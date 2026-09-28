//! Drives `gh stack sync` and describes what it is about to do.
//!
//! Unlike `gh stack submit` (see [`super::submit`]), sync can be run as a
//! plain subprocess and is *not* reproduced step by step here. Per
//! `gh stack sync --help` and the `github/gh-stack` README:
//!
//! - It only prompts when the local and remote stacks have diverged, or when
//!   merged branches could be pruned, and only "in an interactive terminal".
//!   Trellis spawns it with no TTY and no stdin, so it never blocks: a
//!   divergence aborts the sync (with exit success, nothing pushed) and
//!   pruning only happens when `--prune` is passed explicitly.
//! - A rebase conflict restores every branch to its original state and
//!   advises running `gh stack rebase`.
//! - It has no `--json` output, so the outcome is classified from its
//!   human-readable output ("Stack synced" / "Branches synced", plus
//!   divergence and conflict messages). That wording is taken from the docs
//!   and has not been observed against a live remote; see [`classify`].
//!
//! Because the command's exact outcome only becomes known after it runs, the
//! "what will happen" preview ([`SyncPlan`]) is derived from the stack model
//! we already have, not from a dry run (sync has none).

use std::path::Path;

use crate::shell::{Shell, ShellError, ShellOutput};

use super::layer::Layer;
use super::summary::StackSummary;

/// Mirrors the `gh stack sync` flags Trellis exposes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncOptions {
    /// Equivalent to `gh stack sync --prune`: delete local branches whose
    /// pull requests have merged. Opt-in only.
    pub prune: bool,
}

/// What a sync of a stack is expected to touch, for the confirmation modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    pub trunk: String,
    /// Branches that will be rebased: every layer flagged `needs_rebase`
    /// plus every active layer stacked on top of one (the rebase cascades).
    /// Sync only rebases if the trunk actually moved on fetch.
    pub rebase: Vec<String>,
    /// Branches that will be pushed (`--force-with-lease --atomic`): every
    /// layer that is neither merged nor queued.
    pub push: Vec<String>,
    /// Merged branches that `--prune` will delete locally. Empty unless
    /// pruning was requested.
    pub prune: Vec<String>,
    /// Active layers with no pull request. Sync never opens PRs.
    pub without_pull_request: Vec<String>,
}

impl SyncPlan {
    pub fn for_stack(stack: &StackSummary, options: SyncOptions) -> Self {
        let first_stale = stack
            .layers
            .iter()
            .position(|layer| layer.needs_rebase && !layer.is_merged);
        let active = |layer: &&Layer| !layer.is_merged && !layer.is_queued;

        let rebase = match first_stale {
            Some(first) => stack.layers[first..]
                .iter()
                .filter(active)
                .map(|layer| layer.branch.clone())
                .collect(),
            None => Vec::new(),
        };
        let push = stack
            .layers
            .iter()
            .filter(active)
            .map(|layer| layer.branch.clone())
            .collect();
        let prune = if options.prune {
            stack
                .layers
                .iter()
                .filter(|layer| layer.is_merged)
                .map(|layer| layer.branch.clone())
                .collect()
        } else {
            Vec::new()
        };
        let without_pull_request = stack
            .layers
            .iter()
            .filter(active)
            .filter(|layer| layer.pull_request.is_none())
            .map(|layer| layer.branch.clone())
            .collect();

        Self {
            trunk: stack.trunk.clone(),
            rebase,
            push,
            prune,
            without_pull_request,
        }
    }

    /// Whether any layer is known to be behind its base.
    pub fn needs_rebase(&self) -> bool {
        !self.rebase.is_empty()
    }
}

/// How a `gh stack sync` run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// "Stack synced": branches rebased and pushed, and the stack on GitHub
    /// matches the local stack.
    Synced,
    /// "Branches synced": rebased and pushed, but no remote stack object was
    /// created or updated (e.g. fewer than two PRs).
    BranchesSynced,
    /// Local and remote stacks have diverged. In a non-TTY run sync aborts
    /// without pushing anything and still exits successfully.
    Diverged,
    /// A rebase conflict; sync restored all branches and advises
    /// `gh stack rebase`.
    Conflict,
    /// Exited successfully with output we don't recognise.
    Completed,
}

/// Runs `gh stack sync` for the stack that is currently checked out.
///
/// Note that `gh stack sync` always operates on the *current* stack; the
/// caller must ensure the stack of interest is the checked-out one.
///
/// A conflict reported through a failing exit status is returned as
/// [`SyncOutcome::Conflict`] rather than an error; every other failure is
/// propagated.
pub async fn sync_stack(
    shell: &impl Shell,
    repo: &Path,
    remote: &str,
    options: SyncOptions,
) -> Result<SyncOutcome, ShellError> {
    let mut args = vec!["stack", "sync", "--remote", remote];
    if options.prune {
        args.push("--prune");
    }

    match shell.run_long(repo, "gh", &args).await {
        Ok(output) => Ok(classify(&output)),
        Err(ShellError::CommandFailed { output, .. })
            if classify(&output) == SyncOutcome::Conflict =>
        {
            Ok(SyncOutcome::Conflict)
        }
        Err(error) => Err(error),
    }
}

/// Classifies sync output. `gh` writes its status lines to stderr, so both
/// streams are searched.
fn classify(output: &ShellOutput) -> SyncOutcome {
    let text = format!("{}\n{}", output.stdout, output.stderr).to_lowercase();

    if text.contains("conflict") {
        SyncOutcome::Conflict
    } else if text.contains("diverge") {
        SyncOutcome::Diverged
    } else if text.contains("stack synced") {
        SyncOutcome::Synced
    } else if text.contains("branches synced") {
        SyncOutcome::BranchesSynced
    } else {
        SyncOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::MockShell;
    use crate::test_fixtures::stack_summary;

    fn out(stdout: &str, stderr: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: stdout.to_string(),
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

    const ARGS: &[&str] = &["stack", "sync", "--remote", "origin"];
    const PRUNE_ARGS: &[&str] = &["stack", "sync", "--remote", "origin", "--prune"];

    async fn run(shell: &MockShell, prune: bool) -> Result<SyncOutcome, ShellError> {
        sync_stack(shell, Path::new("."), "origin", SyncOptions { prune }).await
    }

    #[tokio::test]
    async fn runs_gh_stack_sync_against_the_remote_without_prune_by_default() {
        let shell = MockShell::new().when("gh", ARGS, out("", "Stack synced\n"));

        assert_eq!(run(&shell, false).await.unwrap(), SyncOutcome::Synced);
        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn passes_prune_only_when_requested() {
        let shell = MockShell::new().when("gh", PRUNE_ARGS, out("Branches synced", ""));

        assert_eq!(
            run(&shell, true).await.unwrap(),
            SyncOutcome::BranchesSynced
        );
    }

    #[tokio::test]
    async fn reports_divergence_even_though_gh_exits_successfully() {
        let shell = MockShell::new().when(
            "gh",
            ARGS,
            out("", "Local and remote stacks have diverged; sync aborted\n"),
        );

        assert_eq!(run(&shell, false).await.unwrap(), SyncOutcome::Diverged);
    }

    #[tokio::test]
    async fn reports_a_conflict_from_a_failing_exit_as_an_outcome() {
        let shell = MockShell::new().when(
            "gh",
            ARGS,
            failed("rebase conflict; run gh stack rebase to resolve"),
        );

        assert_eq!(run(&shell, false).await.unwrap(), SyncOutcome::Conflict);
    }

    #[tokio::test]
    async fn other_failures_propagate() {
        let shell = MockShell::new().when("gh", ARGS, failed("push rejected"));

        assert!(matches!(
            run(&shell, false).await,
            Err(ShellError::CommandFailed { .. })
        ));
    }

    #[tokio::test]
    async fn unrecognised_successful_output_is_completed() {
        let shell = MockShell::new().when("gh", ARGS, out("ok", ""));

        assert_eq!(run(&shell, false).await.unwrap(), SyncOutcome::Completed);
    }

    #[test]
    fn plan_lists_active_branches_to_push_and_none_to_rebase_when_up_to_date() {
        let mut stack = stack_summary("s", 3);
        stack.layers[0].is_merged = true;

        let plan = SyncPlan::for_stack(&stack, SyncOptions::default());

        assert_eq!(plan.push, ["s-layer-1", "s-layer-2"]);
        assert!(!plan.needs_rebase());
        assert!(plan.prune.is_empty());
    }

    #[test]
    fn plan_cascades_the_rebase_to_layers_above_a_stale_one() {
        let mut stack = stack_summary("s", 3);
        stack.layers[1].needs_rebase = true;

        let plan = SyncPlan::for_stack(&stack, SyncOptions::default());

        assert_eq!(plan.rebase, ["s-layer-1", "s-layer-2"]);
        assert!(plan.needs_rebase());
    }

    #[test]
    fn plan_lists_merged_branches_to_prune_only_when_requested() {
        let mut stack = stack_summary("s", 2);
        stack.layers[0].is_merged = true;

        let without = SyncPlan::for_stack(&stack, SyncOptions { prune: false });
        let with = SyncPlan::for_stack(&stack, SyncOptions { prune: true });

        assert!(without.prune.is_empty());
        assert_eq!(with.prune, ["s-layer-0"]);
    }
}
