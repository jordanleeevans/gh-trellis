//! Merges a stack of pull requests via `gh stack merge`.
//!
//! Behaviour verified against the `github/gh-stack` source (`cmd/merge.go`
//! at commit `68ce60c`, whose help text is identical to the installed
//! v0.1.1's `gh stack merge --help`) and the project README's
//! "`gh stack merge`" section. No live merge was run.
//!
//! - **What merges.** GitHub's atomic stack merge: "All members of the stack
//!   up to and including your chosen pull request are merged into the base
//!   branch in a single, all-or-nothing operation: if any PR cannot be
//!   merged, none are" (help text; `merge.go` L48-51).
//! - **Which PRs, in what order.** `mergeCandidates` (`merge.go` L504-524)
//!   walks the stack's PRs bottom to top, *skipping* already-merged PRs and
//!   stopping at the first draft or closed PR, which it returns as the
//!   "blocker". With no argument, in a non-interactive run, the whole stack
//!   is merged: `runMerge` refuses outright if there is a blocker ("cannot
//!   merge the whole stack: pull request #N is a draft", L172-181) rather
//!   than merging only the part below it, and otherwise targets the top
//!   candidate. [`merge_plan`] reproduces exactly this selection.
//! - **Which stack.** With no argument it is the stack of the *checked-out
//!   branch*, read from local tracking, then re-fetched from GitHub
//!   (`resolveActiveRemoteStack`, L235-295). PR state always comes from
//!   GitHub, never from local state. Trellis passes no argument, so callers
//!   must only target the checked-out stack. (A bare number argument is
//!   tried first as a *stack* number and only then as a PR number, L208-232,
//!   so passing a PR number to limit the merge could silently target a
//!   different, whole stack; trellis never does that.)
//! - **Checks.** "Only basic pull request state is checked before merging
//!   (open and not a draft); GitHub evaluates branch protection and
//!   repository rules when the merge runs" (help text, L64-67). gh-stack
//!   does *not* refuse pending or failing checks itself, so neither does
//!   trellis; a failing required check makes GitHub reject the whole merge.
//! - **Merge method.** `--merge`, `--squash`, `--rebase` or
//!   `--merge-method <m>`; more than one distinct method is an error
//!   (`resolveMergeMethodFlag`, L576-608). With none given, a non-interactive
//!   run uses the viewer's last-used method (falling back to the first
//!   allowed one, L184-189), which trellis can't show in advance, so it
//!   always passes the method explicitly. A method the repository disallows
//!   is an error (L156-159). If the base branch uses a merge queue the
//!   method is ignored with a warning and the PRs are enqueued instead
//!   (L138-144); the README adds that queued PRs "may land in separate
//!   groups rather than all at once".
//! - **Prompting and `--yes`.** The wizard runs only when
//!   `cfg.IsInteractive() && !opts.yes` (L162), and `IsInteractive` is
//!   "stdout is a terminal" (`internal/config/config.go` L174-176).
//!   [`crate::shell::ProcessShell`] pipes stdout and gives no stdin, so the
//!   command would already merge *without any prompt* even without `--yes`.
//!   Trellis passes `--yes` anyway, explicitly, and only from the effect that
//!   runs after the user has typed the phrase in trellis's own `danger`
//!   confirmation, which names every PR and the method. Passing it pins the
//!   headless code path: if stdout were ever treated as a terminal (e.g.
//!   `GH_FORCE_TTY` in the environment) the wizard would otherwise start
//!   with no usable input. `--yes` therefore never skips a confirmation the
//!   user would have seen; trellis's modal *is* the confirmation.
//! - **Afterwards.** The command runs no git operations: it neither rebases
//!   nor deletes local branches, and since the whole stack merges there are
//!   no remaining open layers to retarget. `gh stack sync` updates local
//!   state afterwards.
//! - **Reporting.** All output goes to stderr (`config.go` L133-151). On
//!   success: "✓ Merged #1, #2 into main (abc1234)", or "✓ Added #1, #2 to
//!   the merge queue for main". On failure: "✗ merge failed: <message>"
//!   followed by "Stack merges are atomic, so nothing was merged." (non-zero
//!   exit). There is no partial success to report per PR, except that
//!   merge-queue PRs may land separately. It polls for up to 600 × 1s
//!   (L405-438) and then warns "Merge is still in progress".

use std::path::Path;

use crate::shell::{Shell, ShellError, ShellOutput};

use super::summary::StackSummary;

/// The merge method, always passed to `gh stack merge` explicitly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeMethod {
    /// `--merge`: a merge commit (GitHub's default merge button).
    #[default]
    Merge,
    /// `--squash`: squash and merge.
    Squash,
    /// `--rebase`: rebase and merge.
    Rebase,
}

impl MergeMethod {
    /// The `gh stack merge` flag for this method.
    pub fn flag(self) -> &'static str {
        match self {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }

    /// The next method in the merge → squash → rebase cycle.
    pub fn next(self) -> Self {
        match self {
            MergeMethod::Merge => MergeMethod::Squash,
            MergeMethod::Squash => MergeMethod::Rebase,
            MergeMethod::Rebase => MergeMethod::Merge,
        }
    }
}

/// One pull request a merge will act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCandidate {
    pub number: u64,
    pub title: Option<String>,
    pub branch: String,
}

/// Exactly what `gh stack merge` (no argument, non-interactive) will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    /// The stack's base branch everything merges into.
    pub trunk: String,
    /// The pull requests that merge, bottom to top.
    pub prs: Vec<MergeCandidate>,
    /// Already-merged pull requests, skipped as gh-stack skips them.
    pub already_merged: Vec<MergeCandidate>,
    /// Layers above the merged PRs with no pull request; not merged.
    pub not_submitted: Vec<String>,
}

impl MergePlan {
    pub fn pr_numbers(&self) -> Vec<u64> {
        self.prs.iter().map(|pr| pr.number).collect()
    }
}

/// Why a PR stops the merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    Draft,
    Closed,
}

impl BlockReason {
    pub fn describe(self) -> &'static str {
        match self {
            BlockReason::Draft => "a draft",
            BlockReason::Closed => "closed",
        }
    }
}

/// Why a stack can't be merged as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeRefusal {
    /// The stack has no layers.
    Empty,
    /// Every pull request is already merged (gh-stack: "This stack is
    /// already fully merged.").
    AlreadyMerged,
    /// No layer has an open pull request.
    NothingToMerge,
    /// A draft or closed PR sits in the stack; gh-stack refuses to merge the
    /// whole stack ("cannot merge the whole stack: pull request #N is ...").
    Blocked {
        number: u64,
        branch: String,
        reason: BlockReason,
    },
    /// A layer without a pull request sits below one that would merge.
    /// This is trellis's own check, not gh-stack's: the stack on GitHub is
    /// built from pull requests, so it can't match the local stack here and
    /// trellis can't say exactly what would merge.
    UnsubmittedBelow { branch: String },
}

/// Computes the ordered pull requests a merge of `stack` will act on,
/// mirroring gh-stack's `mergeCandidates` plus the whole-stack blocker check
/// of its non-interactive path (see the module docs).
///
/// PR state comes from trellis's last refresh; gh-stack re-reads it from
/// GitHub when it runs, and the caller re-checks this plan just before
/// running the merge.
pub fn merge_plan(stack: &StackSummary) -> Result<MergePlan, MergeRefusal> {
    if stack.layers.is_empty() {
        return Err(MergeRefusal::Empty);
    }

    let mut prs = Vec::new();
    let mut already_merged = Vec::new();
    let mut not_submitted: Vec<String> = Vec::new();

    for layer in &stack.layers {
        let Some(pr) = &layer.pull_request else {
            not_submitted.push(layer.branch.clone());
            continue;
        };
        let candidate = MergeCandidate {
            number: pr.number,
            title: pr.title.clone(),
            branch: layer.branch.clone(),
        };

        if pr.state.eq_ignore_ascii_case("MERGED") || layer.is_merged {
            already_merged.push(candidate);
            continue;
        }

        let blocked = if pr.is_draft == Some(true) {
            Some(BlockReason::Draft)
        } else if pr.state.eq_ignore_ascii_case("CLOSED") {
            Some(BlockReason::Closed)
        } else {
            None
        };
        if let Some(reason) = blocked {
            return Err(MergeRefusal::Blocked {
                number: pr.number,
                branch: layer.branch.clone(),
                reason,
            });
        }

        if let Some(branch) = not_submitted.first() {
            return Err(MergeRefusal::UnsubmittedBelow {
                branch: branch.clone(),
            });
        }
        prs.push(candidate);
    }

    if prs.is_empty() {
        return Err(if already_merged.is_empty() {
            MergeRefusal::NothingToMerge
        } else {
            MergeRefusal::AlreadyMerged
        });
    }

    Ok(MergePlan {
        trunk: stack.trunk.clone(),
        prs,
        already_merged,
        not_submitted,
    })
}

/// The `gh` arguments for a confirmed merge. No stack or PR number: the
/// checked-out stack is merged as a whole. `--yes` is only ever passed from
/// here, which only runs after trellis's own danger confirmation.
pub fn merge_args(method: MergeMethod) -> Vec<&'static str> {
    vec!["stack", "merge", "--yes", method.flag()]
}

/// How a successful `gh stack merge` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// "Merged #1, #2 into main (abc1234)".
    Merged {
        prs: Vec<u64>,
        base: String,
        sha: Option<String>,
    },
    /// "Added #1, #2 to the merge queue for main".
    Enqueued { prs: Vec<u64>, base: String },
    /// "This stack is already fully merged."
    AlreadyMerged,
    /// Exited successfully with output we don't recognise.
    Completed,
}

/// Why a `gh stack merge` run failed.
#[derive(Debug)]
pub enum MergeFailure {
    /// GitHub rejected the atomic merge ("merge failed: ..."), so nothing
    /// was merged.
    Rejected { message: String },
    /// The merge was started but hadn't finished when gh-stack (or trellis)
    /// stopped waiting. It may still complete on GitHub.
    StillInProgress,
    /// gh-stack refused before merging (the first "✗" line of its output),
    /// e.g. a draft in the stack or a disallowed merge method.
    Refused { message: String },
    /// Any other failure to run the command.
    Shell(ShellError),
}

/// Runs `gh stack merge --yes <method>` for the checked-out stack.
///
/// The caller must have shown the user [`merge_plan`] in a danger
/// confirmation and checked the stack is the checked-out one.
pub async fn merge_stack(
    shell: &impl Shell,
    repo: &Path,
    method: MergeMethod,
) -> Result<MergeOutcome, MergeFailure> {
    match shell.run_long(repo, "gh", &merge_args(method)).await {
        Ok(output) => Ok(classify_success(&output)),
        Err(ShellError::CommandFailed { program, output }) => {
            Err(classify_failure(program, output))
        }
        Err(ShellError::Timeout { .. }) => Err(MergeFailure::StillInProgress),
        Err(error) => Err(MergeFailure::Shell(error)),
    }
}

/// Output lines with gh-stack's leading status glyph removed.
fn lines(output: &ShellOutput) -> impl Iterator<Item = &str> {
    output
        .stderr
        .lines()
        .chain(output.stdout.lines())
        .map(|line| {
            line.trim()
                .trim_start_matches(['\u{2713}', '\u{2717}', '\u{26a0}', '\u{2139}'])
                .trim()
        })
}

fn classify_success(output: &ShellOutput) -> MergeOutcome {
    for line in lines(output) {
        if let Some(rest) = line.strip_prefix("Merged ")
            && let Some((list, tail)) = rest.split_once(" into ")
        {
            let (base, sha) = match tail.split_once(" (") {
                Some((base, sha)) => (base, Some(sha.trim_end_matches(')').to_string())),
                None => (tail, None),
            };
            return MergeOutcome::Merged {
                prs: pr_numbers(list),
                base: base.trim().to_string(),
                sha,
            };
        }
        if let Some(rest) = line.strip_prefix("Added ")
            && let Some((list, base)) = rest.split_once(" to the merge queue for ")
        {
            return MergeOutcome::Enqueued {
                prs: pr_numbers(list),
                base: base.trim().to_string(),
            };
        }
        if line.contains("already fully merged") {
            return MergeOutcome::AlreadyMerged;
        }
    }
    MergeOutcome::Completed
}

fn classify_failure(program: String, output: ShellOutput) -> MergeFailure {
    for line in lines(&output) {
        if let Some(message) = line.strip_prefix("merge failed: ") {
            return MergeFailure::Rejected {
                message: message.to_string(),
            };
        }
        if line.contains("still in progress") {
            return MergeFailure::StillInProgress;
        }
    }
    // gh-stack prefixes its own errors with "✗"; anything else (e.g. a raw
    // `gh` failure) is left to the generic shell-error wording.
    let first_error = output
        .stderr
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix('\u{2717}'))
        .map(|message| message.trim().to_string());
    match first_error {
        Some(message) => MergeFailure::Refused { message },
        None => MergeFailure::Shell(ShellError::CommandFailed { program, output }),
    }
}

/// Every `#123` in `text`, in order.
fn pr_numbers(text: &str) -> Vec<u64> {
    text.split('#')
        .skip(1)
        .filter_map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::MockShell;
    use crate::stack::PullRequestRef;
    use crate::test_fixtures::stack_summary;

    fn pr(number: u64, state: &str, draft: bool) -> PullRequestRef {
        PullRequestRef {
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            state: state.to_string(),
            title: Some(format!("PR {number}")),
            is_draft: Some(draft),
            checks_status: None,
            review_decision: None,
        }
    }

    /// A stack whose layers have the given PRs (`None` = unsubmitted).
    fn stack(prs: Vec<Option<PullRequestRef>>) -> StackSummary {
        let mut stack = stack_summary("s", prs.len());
        for (layer, pr) in stack.layers.iter_mut().zip(prs) {
            layer.is_merged = pr.as_ref().is_some_and(|pr| pr.state == "MERGED");
            layer.pull_request = pr;
        }
        stack
    }

    fn numbers(plan: &MergePlan) -> Vec<u64> {
        plan.pr_numbers()
    }

    #[test]
    fn plan_merges_every_open_pr_bottom_to_top() {
        let plan = merge_plan(&stack(vec![
            Some(pr(44, "OPEN", false)),
            Some(pr(45, "OPEN", false)),
            Some(pr(46, "OPEN", false)),
        ]))
        .unwrap();

        assert_eq!(numbers(&plan), [44, 45, 46]);
        assert_eq!(plan.trunk, "main");
        assert_eq!(plan.prs[0].branch, "s-layer-0");
        assert_eq!(plan.prs[0].title.as_deref(), Some("PR 44"));
        assert!(plan.already_merged.is_empty());
    }

    #[test]
    fn plan_skips_already_merged_prs() {
        let plan = merge_plan(&stack(vec![
            Some(pr(1, "MERGED", false)),
            Some(pr(2, "OPEN", false)),
            Some(pr(3, "OPEN", false)),
        ]))
        .unwrap();

        assert_eq!(numbers(&plan), [2, 3]);
        assert_eq!(plan.already_merged[0].number, 1);
    }

    #[test]
    fn plan_refuses_the_whole_stack_when_a_draft_is_anywhere_in_it() {
        for draft_at in 0..3 {
            let prs = (0..3)
                .map(|i| Some(pr(10 + i, "OPEN", i == draft_at)))
                .collect();

            assert_eq!(
                merge_plan(&stack(prs)),
                Err(MergeRefusal::Blocked {
                    number: 10 + draft_at,
                    branch: format!("s-layer-{draft_at}"),
                    reason: BlockReason::Draft,
                })
            );
        }
    }

    #[test]
    fn plan_refuses_a_closed_pr() {
        assert!(matches!(
            merge_plan(&stack(vec![
                Some(pr(1, "OPEN", false)),
                Some(pr(2, "CLOSED", false)),
            ])),
            Err(MergeRefusal::Blocked {
                number: 2,
                reason: BlockReason::Closed,
                ..
            })
        ));
    }

    #[test]
    fn plan_lists_unsubmitted_top_layers_without_merging_them() {
        let plan = merge_plan(&stack(vec![Some(pr(1, "OPEN", false)), None, None])).unwrap();

        assert_eq!(numbers(&plan), [1]);
        assert_eq!(plan.not_submitted, ["s-layer-1", "s-layer-2"]);
    }

    #[test]
    fn plan_refuses_an_unsubmitted_layer_below_an_open_pr() {
        assert_eq!(
            merge_plan(&stack(vec![None, Some(pr(2, "OPEN", false))])),
            Err(MergeRefusal::UnsubmittedBelow {
                branch: "s-layer-0".to_string()
            })
        );
    }

    #[test]
    fn plan_reports_fully_merged_empty_and_unsubmitted_stacks() {
        assert_eq!(
            merge_plan(&stack(vec![Some(pr(1, "MERGED", false))])),
            Err(MergeRefusal::AlreadyMerged)
        );
        assert_eq!(
            merge_plan(&stack(vec![None, None])),
            Err(MergeRefusal::NothingToMerge)
        );
        assert_eq!(merge_plan(&stack(vec![])), Err(MergeRefusal::Empty));
    }

    #[test]
    fn method_cycles_and_maps_to_flags() {
        assert_eq!(MergeMethod::default(), MergeMethod::Merge);
        assert_eq!(MergeMethod::Merge.next(), MergeMethod::Squash);
        assert_eq!(MergeMethod::Squash.next(), MergeMethod::Rebase);
        assert_eq!(MergeMethod::Rebase.next(), MergeMethod::Merge);
        assert_eq!(
            merge_args(MergeMethod::Squash),
            ["stack", "merge", "--yes", "--squash"]
        );
    }

    #[test]
    fn args_never_carry_a_stack_or_pr_number() {
        for method in [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase] {
            let args = merge_args(method);
            assert!(!args.iter().any(|a| a.parse::<u64>().is_ok()));
            assert_eq!(args.iter().filter(|a| a.starts_with("--")).count(), 2);
        }
    }

    fn out(stderr: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: 0,
        })
    }

    fn failed(stderr: &str) -> Result<ShellOutput, ShellError> {
        Err(ShellError::CommandFailed {
            program: "gh".to_string(),
            output: ShellOutput {
                stdout: String::new(),
                stderr: stderr.to_string(),
                exit_code: 1,
            },
        })
    }

    const SQUASH: &[&str] = &["stack", "merge", "--yes", "--squash"];

    async fn run(shell: &MockShell) -> Result<MergeOutcome, MergeFailure> {
        merge_stack(shell, Path::new("."), MergeMethod::Squash).await
    }

    #[tokio::test]
    async fn reports_the_merged_prs_base_and_sha() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            out("Merging #44, #45 into main via squash...\n\u{2713} Merged #44, #45 into main (abc1234)\n"),
        );

        assert_eq!(
            run(&shell).await.unwrap(),
            MergeOutcome::Merged {
                prs: vec![44, 45],
                base: "main".to_string(),
                sha: Some("abc1234".to_string()),
            }
        );
        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn reports_a_merge_without_a_sha() {
        let shell = MockShell::new().when("gh", SQUASH, out("\u{2713} Merged #7 into develop\n"));

        assert_eq!(
            run(&shell).await.unwrap(),
            MergeOutcome::Merged {
                prs: vec![7],
                base: "develop".to_string(),
                sha: None,
            }
        );
    }

    #[tokio::test]
    async fn reports_prs_added_to_a_merge_queue() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            out("\u{26a0} the base branch \"main\" uses a merge queue; ignoring the merge method\n\u{2713} Added #1, #2 to the merge queue for main\nThey will merge once the queue processes them.\n"),
        );

        assert_eq!(
            run(&shell).await.unwrap(),
            MergeOutcome::Enqueued {
                prs: vec![1, 2],
                base: "main".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn reports_an_already_merged_stack_and_unknown_success() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            out("\u{2713} This stack is already fully merged.\n"),
        );
        assert_eq!(run(&shell).await.unwrap(), MergeOutcome::AlreadyMerged);

        let shell = MockShell::new().when("gh", SQUASH, out("done"));
        assert_eq!(run(&shell).await.unwrap(), MergeOutcome::Completed);
    }

    #[tokio::test]
    async fn a_rejected_atomic_merge_carries_githubs_message() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            failed("Merging #44, #45 into main via squash...\n\u{2717} merge failed: Required status check \"ci\" is failing on #45\nStack merges are atomic, so nothing was merged.\n"),
        );

        let Err(MergeFailure::Rejected { message }) = run(&shell).await else {
            panic!("expected a rejection");
        };
        assert_eq!(message, "Required status check \"ci\" is failing on #45");
    }

    #[tokio::test]
    async fn a_refusal_before_merging_carries_the_first_error_line() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            failed("\u{2717} cannot merge the whole stack: pull request #45 is a draft\nMerge up to #44 with `gh stack merge 44`\n"),
        );

        let Err(MergeFailure::Refused { message }) = run(&shell).await else {
            panic!("expected a refusal");
        };
        assert_eq!(
            message,
            "cannot merge the whole stack: pull request #45 is a draft"
        );
    }

    #[tokio::test]
    async fn still_in_progress_and_timeouts_are_not_reported_as_failures_to_merge() {
        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            failed("\u{26a0} Merge is still in progress. Check the pull requests on GitHub.\n"),
        );
        assert!(matches!(
            run(&shell).await,
            Err(MergeFailure::StillInProgress)
        ));

        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            Err(ShellError::Timeout {
                program: "gh".to_string(),
                timeout: std::time::Duration::from_secs(300),
            }),
        );
        assert!(matches!(
            run(&shell).await,
            Err(MergeFailure::StillInProgress)
        ));
    }

    #[tokio::test]
    async fn other_failures_propagate_as_shell_errors() {
        let shell = MockShell::new().when("gh", SQUASH, failed("HTTP 401: Bad credentials"));
        assert!(matches!(
            run(&shell).await,
            Err(MergeFailure::Shell(ShellError::CommandFailed { .. }))
        ));

        let shell = MockShell::new().when(
            "gh",
            SQUASH,
            Err(ShellError::BinaryNotFound("gh".to_string())),
        );
        assert!(matches!(
            run(&shell).await,
            Err(MergeFailure::Shell(ShellError::BinaryNotFound(_)))
        ));
    }
}
