//! Drives `gh stack submit` layer by layer.
//!
//! `gh stack submit` itself has no `--json`/structured-output mode (checked
//! against `github/gh-stack`'s README as of this writing) and, without
//! `--auto`, opens a full-screen interactive editor — not something a
//! subprocess spawned from inside this TUI can drive or parse output from.
//! So rather than shelling out to `gh stack submit` and trying to scrape its
//! human-readable output for per-layer results, this module reproduces its
//! effect (push each branch, create or update each branch's pull request)
//! one layer at a time via plain `git`/`gh` calls whose exit code and output
//! we control and can report on individually. That's what lets the caller
//! (see [`submit_stack`]) report a clear
//! push/created/updated/failed result per layer instead of a single
//! pass/fail for the whole stack.

use std::path::Path;

use crate::shell::{Shell, ShellError};

use super::layer::Layer;

/// Mirrors the real `gh stack submit --auto`/`--open` flags.
///
/// Per `gh stack submit --help`/README: in the interactive editor (no
/// `--auto`), new pull requests default to ready-for-review; a user can flip
/// individual ones to draft. With `--auto`, the editor is skipped, titles
/// are auto-generated, and new pull requests default to draft *unless*
/// `--open` is also passed. This module has no interactive editor, so it
/// always uses auto-generated titles (`gh pr create --fill`) and derives the
/// draft/ready default for newly created pull requests from these two flags
/// via [`SubmitOptions::new_pr_is_draft`], matching that same rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubmitOptions {
    /// Equivalent to `gh stack submit --auto`.
    pub auto: bool,
    /// Equivalent to `gh stack submit --open`.
    pub open: bool,
}

impl SubmitOptions {
    /// Whether a newly created pull request should be opened as a draft.
    pub fn new_pr_is_draft(&self) -> bool {
        self.auto && !self.open
    }
}

/// The result of successfully submitting one layer's branch and pull
/// request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitLayerOutcome {
    PullRequestCreated { number: u64 },
    PullRequestUpdated { number: u64 },
}

/// Force-pushes `branch` to `remote`, setting up the upstream tracking ref
/// if it isn't already configured.
///
/// Uses `--force-with-lease` rather than a plain push since stacked layers
/// are routinely rebased as lower layers change.
pub async fn push_layer_branch(
    shell: &impl Shell,
    repo: &Path,
    remote: &str,
    branch: &str,
) -> Result<(), ShellError> {
    let refspec = format!("{branch}:{branch}");
    shell
        .run(
            repo,
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                remote,
                &refspec,
            ],
        )
        .await?;
    Ok(())
}

/// Creates a pull request for `layer` if it doesn't have one yet, or brings
/// an existing one's base branch (and, when requested, its ready/draft
/// state) up to date otherwise.
pub async fn sync_layer_pull_request(
    shell: &impl Shell,
    repo: &Path,
    layer: &Layer,
    options: &SubmitOptions,
) -> Result<SubmitLayerOutcome, ShellError> {
    match &layer.pull_request {
        None => {
            let mut args = vec![
                "pr",
                "create",
                "--head",
                layer.branch.as_str(),
                "--base",
                layer.base.as_str(),
                "--fill",
            ];
            if options.new_pr_is_draft() {
                args.push("--draft");
            }

            let output = shell.run(repo, "gh", &args).await?;
            let number = parse_pr_number(&output.stdout)?;
            Ok(SubmitLayerOutcome::PullRequestCreated { number })
        }
        Some(pr) => {
            let number_arg = pr.number.to_string();
            shell
                .run(
                    repo,
                    "gh",
                    &["pr", "edit", &number_arg, "--base", layer.base.as_str()],
                )
                .await?;

            if options.open && pr.is_draft == Some(true) {
                shell.run(repo, "gh", &["pr", "ready", &number_arg]).await?;
            }

            Ok(SubmitLayerOutcome::PullRequestUpdated { number: pr.number })
        }
    }
}

/// One step of a single layer's progress through [`submit_stack`].
#[derive(Debug)]
pub enum SubmitEvent {
    /// The layer's branch is being pushed.
    Pushing,
    /// The push succeeded; the pull request is being created or updated.
    Pushed,
    /// Both steps succeeded.
    Completed(SubmitLayerOutcome),
    /// The push failed; the pull request step was skipped.
    PushFailed(ShellError),
    /// The push succeeded but creating/updating the pull request failed.
    PullRequestFailed(ShellError),
}

/// Pushes and syncs the pull request for each of `layers` in order,
/// reporting each step via `on_event` (with the index into `layers`).
///
/// A layer whose push or pull request step fails is reported as failed and
/// the loop moves on to the next layer — a failure on one layer never stops
/// the rest of the stack from being submitted, and never gets folded into a
/// single overall pass/fail result.
pub async fn submit_stack(
    shell: &impl Shell,
    repo: &Path,
    remote: &str,
    layers: &[Layer],
    options: SubmitOptions,
    mut on_event: impl FnMut(usize, SubmitEvent),
) {
    for (layer_index, layer) in layers.iter().enumerate() {
        on_event(layer_index, SubmitEvent::Pushing);

        if let Err(error) = push_layer_branch(shell, repo, remote, &layer.branch).await {
            on_event(layer_index, SubmitEvent::PushFailed(error));
            continue;
        }
        on_event(layer_index, SubmitEvent::Pushed);

        let event = match sync_layer_pull_request(shell, repo, layer, &options).await {
            Ok(outcome) => SubmitEvent::Completed(outcome),
            Err(error) => SubmitEvent::PullRequestFailed(error),
        };
        on_event(layer_index, event);
    }
}

/// Parses the pull request number out of `gh pr create`'s stdout, which is
/// the created PR's URL (e.g. `https://github.com/o/r/pull/45`).
fn parse_pr_number(stdout: &str) -> Result<u64, ShellError> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().rsplit('/').next()?.parse::<u64>().ok())
        .ok_or_else(|| {
            ShellError::UnexpectedOutput(format!(
                "could not parse a pull request number from `gh pr create` output: {stdout:?}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use crate::stack::pull_request::PullRequestRef;

    fn ok(stdout: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
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

    fn layer_without_pr(branch: &str, base: &str) -> Layer {
        Layer {
            branch: branch.to_string(),
            head: None,
            base: base.to_string(),
            is_current: false,
            is_merged: false,
            is_queued: false,
            needs_rebase: false,
            pull_request: None,
            commits: Vec::new(),
            position: 0,
        }
    }

    fn layer_with_pr(branch: &str, base: &str, number: u64, is_draft: bool) -> Layer {
        Layer {
            pull_request: Some(PullRequestRef {
                number,
                url: format!("https://github.com/o/r/pull/{number}"),
                state: "OPEN".to_string(),
                title: None,
                is_draft: Some(is_draft),
                checks_status: None,
                review_decision: None,
            }),
            ..layer_without_pr(branch, base)
        }
    }

    #[tokio::test]
    async fn push_layer_branch_force_pushes_with_lease() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                "origin",
                "feature/layer-1:feature/layer-1",
            ],
            ok(""),
        );

        push_layer_branch(&shell, repo.as_path(), "origin", "feature/layer-1")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn push_layer_branch_propagates_failure() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                "origin",
                "feature/layer-1:feature/layer-1",
            ],
            Err(ShellError::CommandFailed {
                program: "git".to_string(),
                output: ShellOutput {
                    stdout: String::new(),
                    stderr: "stale info".to_string(),
                    exit_code: 1,
                },
            }),
        );

        let result = push_layer_branch(&shell, repo.as_path(), "origin", "feature/layer-1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sync_layer_pull_request_creates_a_draft_pr_when_auto_without_open() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "feature/layer-1",
                "--base",
                "main",
                "--fill",
                "--draft",
            ],
            ok("https://github.com/o/r/pull/45\n"),
        );

        let options = SubmitOptions {
            auto: true,
            open: false,
        };
        let outcome = sync_layer_pull_request(&shell, repo.as_path(), &layer, &options)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestCreated { number: 45 }
        );
    }

    #[tokio::test]
    async fn sync_layer_pull_request_creates_a_ready_pr_when_open() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "feature/layer-1",
                "--base",
                "main",
                "--fill",
            ],
            ok("https://github.com/o/r/pull/45"),
        );

        let options = SubmitOptions {
            auto: true,
            open: true,
        };
        let outcome = sync_layer_pull_request(&shell, repo.as_path(), &layer, &options)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestCreated { number: 45 }
        );
    }

    #[tokio::test]
    async fn sync_layer_pull_request_creates_a_ready_pr_by_default_without_auto() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "feature/layer-1",
                "--base",
                "main",
                "--fill",
            ],
            ok("https://github.com/o/r/pull/45"),
        );

        let outcome =
            sync_layer_pull_request(&shell, repo.as_path(), &layer, &SubmitOptions::default())
                .await
                .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestCreated { number: 45 }
        );
    }

    #[tokio::test]
    async fn sync_layer_pull_request_fails_when_pr_create_fails() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "feature/layer-1",
                "--base",
                "main",
                "--fill",
            ],
            failed("could not create pull request"),
        );

        let result =
            sync_layer_pull_request(&shell, repo.as_path(), &layer, &SubmitOptions::default())
                .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sync_layer_pull_request_updates_base_for_an_existing_pr() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_with_pr("feature/layer-2", "feature/layer-1", 44, false);
        let shell = MockShell::new().when(
            "gh",
            &["pr", "edit", "44", "--base", "feature/layer-1"],
            ok(""),
        );

        let outcome =
            sync_layer_pull_request(&shell, repo.as_path(), &layer, &SubmitOptions::default())
                .await
                .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestUpdated { number: 44 }
        );
    }

    #[tokio::test]
    async fn sync_layer_pull_request_marks_an_existing_draft_ready_when_open() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_with_pr("feature/layer-2", "feature/layer-1", 44, true);
        let shell = MockShell::new()
            .when(
                "gh",
                &["pr", "edit", "44", "--base", "feature/layer-1"],
                ok(""),
            )
            .when("gh", &["pr", "ready", "44"], ok(""));

        let options = SubmitOptions {
            auto: false,
            open: true,
        };
        let outcome = sync_layer_pull_request(&shell, repo.as_path(), &layer, &options)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestUpdated { number: 44 }
        );
    }

    #[tokio::test]
    async fn sync_layer_pull_request_does_not_touch_an_already_ready_pr() {
        let repo = std::env::current_dir().unwrap();
        // is_draft: false — if the code tried to call `gh pr ready` anyway,
        // MockShell would panic on the unregistered call.
        let layer = layer_with_pr("feature/layer-2", "feature/layer-1", 44, false);
        let shell = MockShell::new().when(
            "gh",
            &["pr", "edit", "44", "--base", "feature/layer-1"],
            ok(""),
        );

        let options = SubmitOptions {
            auto: false,
            open: true,
        };
        sync_layer_pull_request(&shell, repo.as_path(), &layer, &options)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn push_then_sync_creates_a_pull_request_after_a_successful_push() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        let shell = MockShell::new()
            .when(
                "git",
                &[
                    "push",
                    "--force-with-lease",
                    "--set-upstream",
                    "origin",
                    "feature/layer-1:feature/layer-1",
                ],
                ok(""),
            )
            .when(
                "gh",
                &[
                    "pr",
                    "create",
                    "--head",
                    "feature/layer-1",
                    "--base",
                    "main",
                    "--fill",
                ],
                ok("https://github.com/o/r/pull/45"),
            );

        push_layer_branch(&shell, repo.as_path(), "origin", &layer.branch)
            .await
            .unwrap();
        let outcome =
            sync_layer_pull_request(&shell, repo.as_path(), &layer, &SubmitOptions::default())
                .await
                .unwrap();

        assert_eq!(
            outcome,
            SubmitLayerOutcome::PullRequestCreated { number: 45 }
        );
    }

    #[tokio::test]
    async fn a_layer_whose_push_fails_never_calls_gh() {
        let repo = std::env::current_dir().unwrap();
        let layer = layer_without_pr("feature/layer-1", "main");
        // Only a response for `git push` is registered — if the caller
        // called `gh pr create` anyway after the push failed, MockShell
        // would panic on the unregistered call.
        let shell = MockShell::new().when(
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                "origin",
                "feature/layer-1:feature/layer-1",
            ],
            failed("stale info"),
        );

        let result = push_layer_branch(&shell, repo.as_path(), "origin", &layer.branch).await;

        assert!(result.is_err());
    }

    #[test]
    fn parse_pr_number_reads_the_trailing_number_from_a_pr_url() {
        assert_eq!(
            parse_pr_number("https://github.com/o/r/pull/45\n").unwrap(),
            45
        );
        assert_eq!(
            parse_pr_number("https://github.com/o/r/pull/45").unwrap(),
            45
        );
    }

    #[test]
    fn parse_pr_number_fails_on_unparseable_output() {
        assert!(parse_pr_number("").is_err());
        assert!(parse_pr_number("not a url\n").is_err());
    }

    #[test]
    fn new_pr_is_draft_matches_gh_stack_submits_auto_open_semantics() {
        assert!(
            SubmitOptions {
                auto: true,
                open: false
            }
            .new_pr_is_draft()
        );
        assert!(
            !SubmitOptions {
                auto: true,
                open: true
            }
            .new_pr_is_draft()
        );
        assert!(
            !SubmitOptions {
                auto: false,
                open: false
            }
            .new_pr_is_draft()
        );
        assert!(
            !SubmitOptions {
                auto: false,
                open: true
            }
            .new_pr_is_draft()
        );
    }
}
