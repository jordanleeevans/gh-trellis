//! Drives `gh stack rebase` and recovers from the conflicts it stops on.
//!
//! Verified against `gh stack rebase --help` (installed v0.1.1) and the
//! `github/gh-stack` source: `cmd/rebase.go`, `cmd/utils.go` (exit codes,
//! `cascadeRebase`, `ensureRerere`, `pickRemote`), `cmd/root.go` and
//! `internal/git/git.go` / `gitops.go`:
//!
//! - **Scope.** With no flag every branch in the stack is rebased, bottom to
//!   top, onto its parent (the first onto the freshly fetched trunk).
//!   `--upstack` starts at the checked-out branch and goes to the top;
//!   `--downstack` goes from the bottom to the checked-out branch. The
//!   optional `[branch]` argument only picks *which stack*: the scope is
//!   still measured from the branch git has checked out, so callers must
//!   only offer this for the checked-out stack.
//! - **Prompts.** It prompts only to enable `git rerere` and to choose
//!   between several remotes, and both are skipped unless stdout is a
//!   terminal (`cfg.IsInteractive()`). Trellis runs it with piped output and
//!   no stdin, and passes `--remote` explicitly, so it never blocks.
//! - **Conflict.** It stops at the first conflicting branch, leaves git
//!   mid-rebase on that branch (`.git/rebase-merge` or `rebase-apply`
//!   present, conflicted files unmerged in the index), writes its own
//!   cascade state to `$GIT_DIR/gh-stack-rebase-state`, prints the
//!   conflicted files, and exits with code 3 (`ErrConflict`). Other
//!   failures exit 1 after restoring every branch.
//! - **`--continue`** needs that state file (else "no rebase in
//!   progress"). If git is mid-rebase it runs `git rebase --continue` with
//!   `GIT_EDITOR=true`, so the resolved files must already be staged; then
//!   it rebases the remaining branches and may stop (exit 3) on the next
//!   conflict.
//! - **`--abort`** also needs the state file. It runs `git rebase --abort`,
//!   then resets every stack branch to the commit recorded before the
//!   rebase started and checks out the original branch.
//!
//! Whether a run stopped on a conflict is decided by asking git (see
//! [`crate::git::rebase_state`]), not by matching output text or relying on
//! exit code 3 alone. The same check finds a rebase left over from an
//! earlier session, including a plain `git rebase` started outside
//! trellis; that one is continued and aborted with plain `git`, since
//! `gh stack` has no state for it.

use std::path::Path;

use crate::git;
use crate::shell::{Shell, ShellError};

/// Which part of the checked-out stack to rebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseScope {
    /// `gh stack rebase`: every branch in the stack.
    Stack,
    /// `gh stack rebase --upstack`: the checked-out branch and every branch
    /// above it.
    Upstack,
}

/// Which tool owns an interrupted rebase, and so continues or aborts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseDriver {
    /// `gh stack rebase` left its cascade state: use `gh stack rebase
    /// --continue` / `--abort`, which also handle the remaining branches.
    GhStack,
    /// A plain `git rebase` with no gh-stack state: use `git rebase
    /// --continue` / `--abort`.
    Git,
}

/// One file with an unresolved conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedFile {
    /// Path relative to the repository root.
    pub path: String,
    /// Whether the working-tree file still has `<<<<<<<`/`>>>>>>>` markers.
    /// `false` means it looks resolved and only needs staging.
    pub has_markers: bool,
}

/// An interrupted rebase: what is stopped, and what still conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseConflict {
    pub driver: RebaseDriver,
    /// The branch being rebased, when git recorded it.
    pub branch: Option<String>,
    /// Unmerged files. Empty once every conflict is resolved and staged,
    /// at which point the rebase can be continued.
    pub files: Vec<ConflictedFile>,
}

/// How a rebase, or a `--continue`, ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// Every branch in scope was rebased; nothing is left in progress.
    Completed,
    /// Stopped on a conflict that needs resolving.
    Conflict(RebaseConflict),
}

/// The `gh` arguments for rebasing the checked-out stack.
fn rebase_args(remote: &str, scope: RebaseScope) -> Vec<&str> {
    let mut args = vec!["stack", "rebase"];
    if scope == RebaseScope::Upstack {
        args.push("--upstack");
    }
    args.extend(["--remote", remote]);
    args
}

/// Runs `gh stack rebase` on the checked-out stack.
///
/// A failing run that leaves git mid-rebase is a conflict and is returned
/// as [`RebaseOutcome::Conflict`]; any other failure is propagated.
pub async fn rebase_stack(
    shell: &impl Shell,
    repo: &Path,
    remote: &str,
    scope: RebaseScope,
) -> Result<RebaseOutcome, ShellError> {
    let result = shell
        .run_long(repo, "gh", &rebase_args(remote, scope))
        .await
        .map(drop);
    settle(shell, repo, result).await
}

/// Continues an interrupted rebase with the tool that owns it. Stopping on
/// another conflict (or on files still unmerged) is a
/// [`RebaseOutcome::Conflict`], not an error.
pub async fn continue_rebase(
    shell: &impl Shell,
    repo: &Path,
    driver: RebaseDriver,
) -> Result<RebaseOutcome, ShellError> {
    let result = match driver {
        RebaseDriver::GhStack => shell
            .run_long(repo, "gh", &["stack", "rebase", "--continue"])
            .await
            .map(drop),
        RebaseDriver::Git => git::rebase_continue(shell, repo).await,
    };
    settle(shell, repo, result).await
}

/// Aborts an interrupted rebase with the tool that owns it. For
/// [`RebaseDriver::GhStack`] this restores every branch in the stack.
pub async fn abort_rebase(
    shell: &impl Shell,
    repo: &Path,
    driver: RebaseDriver,
) -> Result<(), ShellError> {
    match driver {
        RebaseDriver::GhStack => {
            shell
                .run_long(repo, "gh", &["stack", "rebase", "--abort"])
                .await?;
            Ok(())
        }
        RebaseDriver::Git => git::rebase_abort(shell, repo).await,
    }
}

/// Looks for a rebase that is stopped (or a gh-stack cascade that is
/// unfinished), e.g. one left over from an earlier session.
pub async fn interrupted_rebase(
    shell: &impl Shell,
    repo: &Path,
) -> Result<Option<RebaseConflict>, ShellError> {
    let state = git::rebase_state(shell, repo).await?;
    if !state.is_interrupted() {
        return Ok(None);
    }

    let files = if state.in_progress {
        git::conflicted_files(shell, repo)
            .await?
            .into_iter()
            .map(|path| ConflictedFile {
                has_markers: git::has_conflict_markers(repo, &path),
                path,
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(Some(RebaseConflict {
        driver: if state.gh_stack_state {
            RebaseDriver::GhStack
        } else {
            RebaseDriver::Git
        },
        branch: state.branch,
        files,
    }))
}

/// Whether staging a file marked it resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutcome {
    Staged,
    /// Not staged: the file still has conflict markers.
    MarkersRemain,
}

/// Marks `path` resolved (`git add`), refusing while it still contains
/// conflict markers so a half-edited file can't be committed by accident.
pub async fn stage_resolved(
    shell: &impl Shell,
    repo: &Path,
    path: &str,
) -> Result<StageOutcome, ShellError> {
    if git::has_conflict_markers(repo, path) {
        return Ok(StageOutcome::MarkersRemain);
    }
    git::stage(shell, repo, path).await?;
    Ok(StageOutcome::Staged)
}

/// Turns the result of a rebase command into an outcome by asking git
/// whether it is left mid-rebase.
async fn settle(
    shell: &impl Shell,
    repo: &Path,
    result: Result<(), ShellError>,
) -> Result<RebaseOutcome, ShellError> {
    match result {
        Ok(()) => Ok(RebaseOutcome::Completed),
        Err(error @ ShellError::CommandFailed { .. }) => {
            match interrupted_rebase(shell, repo).await {
                Ok(Some(conflict)) => Ok(RebaseOutcome::Conflict(conflict)),
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GH_STACK_REBASE_STATE_FILE: &str = "gh-stack-rebase-state";
    use crate::shell::{MockShell, ProcessShell, ShellOutput};
    use crate::test_fixtures::{FIXTURE_FEATURE_TEXT, conflicting_rebase_repo};

    fn ok(stdout: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn failed(program: &str, exit_code: i32, stderr: &str) -> Result<ShellOutput, ShellError> {
        Err(ShellError::CommandFailed {
            program: program.to_string(),
            output: ShellOutput {
                stdout: String::new(),
                stderr: stderr.to_string(),
                exit_code,
            },
        })
    }

    const GIT_PATH_ARGS: &[&str] = &[
        "rev-parse",
        "--git-path",
        "rebase-merge",
        "--git-path",
        "rebase-apply",
        "--git-path",
        GH_STACK_REBASE_STATE_FILE,
    ];
    const CONFLICTED_ARGS: &[&str] = &["diff", "--name-only", "--diff-filter=U", "-z"];

    /// A fake git dir, and the `rev-parse --git-path` listing pointing at it.
    struct GitDir {
        dir: tempfile::TempDir,
    }

    impl GitDir {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn mid_rebase(self, branch: &str) -> Self {
            let merge = self.dir.path().join("rebase-merge");
            std::fs::create_dir(&merge).unwrap();
            std::fs::write(merge.join("head-name"), format!("refs/heads/{branch}\n")).unwrap();
            self
        }

        fn with_gh_stack_state(self) -> Self {
            std::fs::write(self.dir.path().join(GH_STACK_REBASE_STATE_FILE), "{}").unwrap();
            self
        }

        fn listing(&self) -> String {
            ["rebase-merge", "rebase-apply", GH_STACK_REBASE_STATE_FILE]
                .map(|name| self.dir.path().join(name).display().to_string())
                .join("\n")
        }
    }

    #[test]
    fn scope_maps_to_gh_flags() {
        assert_eq!(
            rebase_args("origin", RebaseScope::Stack),
            ["stack", "rebase", "--remote", "origin"]
        );
        assert_eq!(
            rebase_args("up", RebaseScope::Upstack),
            ["stack", "rebase", "--upstack", "--remote", "up"]
        );
    }

    #[tokio::test]
    async fn a_successful_rebase_completes_without_querying_git() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "rebase", "--remote", "origin"],
            ok("All branches in stack rebased locally"),
        );

        let outcome = rebase_stack(&shell, Path::new("."), "origin", RebaseScope::Stack)
            .await
            .unwrap();

        assert_eq!(outcome, RebaseOutcome::Completed);
        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn a_failure_that_leaves_git_mid_rebase_is_a_conflict() {
        let git_dir = GitDir::new().mid_rebase("s/layer-2").with_gh_stack_state();
        let shell = MockShell::new()
            .when(
                "gh",
                &["stack", "rebase", "--upstack", "--remote", "origin"],
                failed("gh", 3, "Rebasing s/layer-2 onto s/layer-1 — conflict"),
            )
            .when("git", GIT_PATH_ARGS, ok(&git_dir.listing()))
            .when("git", CONFLICTED_ARGS, ok("src/lib.rs\0"));

        let outcome = rebase_stack(&shell, Path::new("."), "origin", RebaseScope::Upstack)
            .await
            .unwrap();

        assert_eq!(
            outcome,
            RebaseOutcome::Conflict(RebaseConflict {
                driver: RebaseDriver::GhStack,
                branch: Some("s/layer-2".to_string()),
                files: vec![ConflictedFile {
                    path: "src/lib.rs".to_string(),
                    has_markers: false,
                }],
            })
        );
    }

    #[tokio::test]
    async fn a_failure_with_git_clean_is_an_error_even_if_it_mentions_conflict() {
        let git_dir = GitDir::new();
        let shell = MockShell::new()
            .when(
                "gh",
                &["stack", "rebase", "--remote", "origin"],
                failed("gh", 1, "conflict? could not fetch trunk"),
            )
            .when("git", GIT_PATH_ARGS, ok(&git_dir.listing()));

        let result = rebase_stack(&shell, Path::new("."), "origin", RebaseScope::Stack).await;

        assert!(matches!(result, Err(ShellError::CommandFailed { .. })));
    }

    #[tokio::test]
    async fn continue_and_abort_use_gh_stack_when_it_owns_the_rebase() {
        let shell = MockShell::new()
            .when("gh", &["stack", "rebase", "--continue"], ok(""))
            .when("gh", &["stack", "rebase", "--abort"], ok(""));

        let outcome = continue_rebase(&shell, Path::new("."), RebaseDriver::GhStack)
            .await
            .unwrap();
        abort_rebase(&shell, Path::new("."), RebaseDriver::GhStack)
            .await
            .unwrap();

        assert_eq!(outcome, RebaseOutcome::Completed);
        assert_eq!(shell.calls().len(), 2);
    }

    #[tokio::test]
    async fn continue_stopping_on_the_next_branch_is_a_conflict() {
        let git_dir = GitDir::new().mid_rebase("s/layer-3").with_gh_stack_state();
        let shell = MockShell::new()
            .when(
                "gh",
                &["stack", "rebase", "--continue"],
                failed("gh", 3, "conflict"),
            )
            .when("git", GIT_PATH_ARGS, ok(&git_dir.listing()))
            .when("git", CONFLICTED_ARGS, ok("b.txt\0"));

        let outcome = continue_rebase(&shell, Path::new("."), RebaseDriver::GhStack)
            .await
            .unwrap();

        let RebaseOutcome::Conflict(conflict) = outcome else {
            panic!("expected a conflict, got {outcome:?}");
        };
        assert_eq!(conflict.branch.as_deref(), Some("s/layer-3"));
        assert_eq!(conflict.files[0].path, "b.txt");
    }

    #[tokio::test]
    async fn a_gh_stack_cascade_with_git_no_longer_mid_rebase_is_still_interrupted() {
        let git_dir = GitDir::new().with_gh_stack_state();
        let shell = MockShell::new().when("git", GIT_PATH_ARGS, ok(&git_dir.listing()));

        let conflict = interrupted_rebase(&shell, Path::new("."))
            .await
            .unwrap()
            .expect("the gh-stack state file alone means continue is needed");

        assert_eq!(conflict.driver, RebaseDriver::GhStack);
        assert!(conflict.files.is_empty());
        assert_eq!(shell.calls().len(), 1, "no unmerged files to list");
    }

    // Against a real repository stopped on a real conflict. gh stack can't
    // run offline, so the fixture uses plain `git rebase`, which this module
    // drives through `RebaseDriver::Git`.

    #[tokio::test]
    async fn detects_a_real_interrupted_rebase_as_a_git_driven_conflict() {
        let dir = conflicting_rebase_repo();

        let conflict = interrupted_rebase(&ProcessShell, dir.path())
            .await
            .unwrap()
            .expect("the fixture is mid-rebase");

        assert_eq!(
            conflict,
            RebaseConflict {
                driver: RebaseDriver::Git,
                branch: Some("feature".to_string()),
                files: vec![ConflictedFile {
                    path: "shared.txt".to_string(),
                    has_markers: true,
                }],
            }
        );
    }

    #[tokio::test]
    async fn staging_is_refused_until_the_markers_are_gone() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();

        assert_eq!(
            stage_resolved(&ProcessShell, repo, "shared.txt")
                .await
                .unwrap(),
            StageOutcome::MarkersRemain
        );
        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        assert_eq!(
            stage_resolved(&ProcessShell, repo, "shared.txt")
                .await
                .unwrap(),
            StageOutcome::Staged
        );

        let conflict = interrupted_rebase(&ProcessShell, repo)
            .await
            .unwrap()
            .unwrap();
        assert!(conflict.files.is_empty(), "ready to continue");
    }

    #[tokio::test]
    async fn continuing_a_real_rebase_before_resolving_reports_the_conflict_again() {
        let dir = conflicting_rebase_repo();

        let outcome = continue_rebase(&ProcessShell, dir.path(), RebaseDriver::Git)
            .await
            .unwrap();

        let RebaseOutcome::Conflict(conflict) = outcome else {
            panic!("expected the conflict to remain, got {outcome:?}");
        };
        assert_eq!(conflict.files[0].path, "shared.txt");
    }

    #[tokio::test]
    async fn resolving_and_continuing_a_real_rebase_completes_it() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();
        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        stage_resolved(&ProcessShell, repo, "shared.txt")
            .await
            .unwrap();

        let outcome = continue_rebase(&ProcessShell, repo, RebaseDriver::Git)
            .await
            .unwrap();

        assert_eq!(outcome, RebaseOutcome::Completed);
        assert_eq!(interrupted_rebase(&ProcessShell, repo).await.unwrap(), None);
    }

    #[tokio::test]
    async fn aborting_a_real_rebase_restores_the_branch() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();

        abort_rebase(&ProcessShell, repo, RebaseDriver::Git)
            .await
            .unwrap();

        assert_eq!(interrupted_rebase(&ProcessShell, repo).await.unwrap(), None);
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            FIXTURE_FEATURE_TEXT
        );
    }
}
