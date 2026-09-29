//! `git rebase` and the git-level state of an interrupted rebase.
//!
//! Conflict detection asks git instead of scraping `gh stack rebase`
//! output, the same way `gh stack` decides for itself
//! (`internal/git/gitops.go` in `github/gh-stack`):
//!
//! - a rebase is in progress when `$GIT_DIR/rebase-merge` or
//!   `$GIT_DIR/rebase-apply` exists (`IsRebaseInProgress`);
//! - the conflicted files are `git diff --name-only --diff-filter=U`
//!   (`ConflictedFiles`).
//!
//! Paths inside the git dir are resolved with `git rev-parse --git-path`,
//! so linked worktrees (where `.git` is a file) work too.

use std::path::{Path, PathBuf};

use crate::shell::{Shell, ShellError};

/// The file `gh stack rebase` writes into the git dir when it stops on a
/// conflict (`rebaseStateFile` in `cmd/rebase.go`). `gh stack rebase
/// --continue` and `--abort` refuse to run without it.
pub const GH_STACK_REBASE_STATE_FILE: &str = "gh-stack-rebase-state";

/// Written by `gh stack modify` while it applies a restructure, and kept
/// when it stops on a conflict (`internal/modify/state.go` in gh-stack).
pub const GH_STACK_MODIFY_STATE_FILE: &str = "gh-stack-modify-state";

/// What the git dir says about an in-progress rebase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebaseState {
    /// `rebase-merge` or `rebase-apply` exists: git is stopped mid-rebase.
    pub in_progress: bool,
    /// `gh stack rebase` left its cascade state behind, so the rebase has
    /// to be continued or aborted through `gh stack`.
    pub gh_stack_state: bool,
    /// `gh stack modify` stopped partway (usually on a conflict), so it has
    /// to be continued or aborted through `gh stack modify`.
    pub gh_stack_modify_state: bool,
    /// The branch being rebased, from `head-name` in the rebase dir
    /// (`refs/heads/` stripped), when git recorded one.
    pub branch: Option<String>,
}

impl RebaseState {
    /// Whether anything is left to continue or abort.
    pub fn is_interrupted(&self) -> bool {
        self.in_progress || self.gh_stack_state || self.gh_stack_modify_state
    }
}

/// Rebases the current branch of the repository at `repo` onto `onto`.
/// Only tests use this, to put fixture repos mid-rebase; the app rebases
/// through `gh stack rebase` (see `crate::stack`).
#[cfg(test)]
pub async fn rebase(shell: &impl Shell, repo: &Path, onto: &str) -> Result<String, ShellError> {
    let output = shell.run(repo, "git", &["rebase", onto]).await?;
    Ok(output.stdout)
}

/// Reads the rebase state of the repository at `repo` from its git dir.
pub async fn rebase_state(shell: &impl Shell, repo: &Path) -> Result<RebaseState, ShellError> {
    let output = shell
        .run(
            repo,
            "git",
            &[
                "rev-parse",
                "--git-path",
                "rebase-merge",
                "--git-path",
                "rebase-apply",
                "--git-path",
                GH_STACK_REBASE_STATE_FILE,
                "--git-path",
                GH_STACK_MODIFY_STATE_FILE,
            ],
        )
        .await?;
    let paths: Vec<PathBuf> = output
        .stdout
        .lines()
        .map(|line| repo.join(line.trim()))
        .collect();
    let [
        rebase_merge,
        rebase_apply,
        gh_stack_state,
        gh_stack_modify_state,
    ] = paths.as_slice()
    else {
        return Err(ShellError::UnexpectedOutput(format!(
            "git rev-parse --git-path printed {:?}",
            output.stdout
        )));
    };

    let dirs = [rebase_merge, rebase_apply];
    let branch = dirs.iter().find_map(|dir| {
        let name = std::fs::read_to_string(dir.join("head-name")).ok()?;
        let name = name.trim();
        let name = name.strip_prefix("refs/heads/").unwrap_or(name);
        (!name.is_empty() && name != "detached HEAD").then(|| name.to_string())
    });

    Ok(RebaseState {
        in_progress: dirs.iter().any(|dir| dir.is_dir()),
        gh_stack_state: gh_stack_state.is_file(),
        gh_stack_modify_state: gh_stack_modify_state.is_file(),
        branch,
    })
}

/// Files with unresolved conflicts (unmerged index entries), as
/// `git diff --name-only --diff-filter=U` reports them. `-z` keeps unusual
/// paths unquoted.
pub async fn conflicted_files(shell: &impl Shell, repo: &Path) -> Result<Vec<String>, ShellError> {
    let output = shell
        .run(
            repo,
            "git",
            &["diff", "--name-only", "--diff-filter=U", "-z"],
        )
        .await?;
    Ok(output
        .stdout
        .split('\0')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

/// Whether the file at `path` (relative to `repo`) still contains conflict
/// markers. A file that can't be read (e.g. deleted on one side) has none.
pub fn has_conflict_markers(repo: &Path, path: &str) -> bool {
    let Ok(bytes) = std::fs::read(repo.join(path)) else {
        return false;
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> "))
}

/// Marks `path` resolved by staging it (`git add -- <path>`). Since git 2.0
/// this also stages a deletion.
pub async fn stage(shell: &impl Shell, repo: &Path, path: &str) -> Result<(), ShellError> {
    shell.run(repo, "git", &["add", "--", path]).await?;
    Ok(())
}

/// `git rebase --continue` without opening an editor for the commit
/// message. `GIT_EDITOR` outranks `core.editor` and `$EDITOR`, so it is set
/// through `env` rather than `-c core.editor=true`. `gh stack rebase
/// --continue` sets the same `GIT_EDITOR=true` (`rebaseContinueOnce` in
/// `internal/git/git.go`).
pub async fn rebase_continue(shell: &impl Shell, repo: &Path) -> Result<(), ShellError> {
    shell
        .run_long(
            repo,
            "env",
            &["GIT_EDITOR=true", "git", "rebase", "--continue"],
        )
        .await?;
    Ok(())
}

/// `git rebase --abort`: puts the branch being rebased back where it was.
pub async fn rebase_abort(shell: &impl Shell, repo: &Path) -> Result<(), ShellError> {
    shell.run_long(repo, "git", &["rebase", "--abort"]).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ProcessShell, ShellOutput};

    #[tokio::test]
    async fn returns_shell_stdout() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &["rebase", "main"],
            Ok(ShellOutput {
                stdout: "Successfully rebased and updated refs/heads/feature.".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let output = rebase(&shell, repo.as_path(), "main").await.unwrap();

        assert_eq!(
            output,
            "Successfully rebased and updated refs/heads/feature."
        );
    }

    async fn run(shell: &ProcessShell, repo: &Path, args: &[&str]) {
        shell.run(repo, "git", args).await.unwrap();
    }

    #[tokio::test]
    async fn rebases_feature_branch_onto_main() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = temp_dir.path();
        let shell = ProcessShell;

        run(&shell, repo, &["init", "-b", "main"]).await;
        run(&shell, repo, &["config", "user.email", "test@example.com"]).await;
        run(&shell, repo, &["config", "user.name", "Test"]).await;
        run(
            &shell,
            repo,
            &["commit", "--allow-empty", "-m", "main commit"],
        )
        .await;
        run(&shell, repo, &["checkout", "-b", "feature"]).await;
        run(
            &shell,
            repo,
            &["commit", "--allow-empty", "-m", "feature commit"],
        )
        .await;
        run(&shell, repo, &["checkout", "main"]).await;
        run(
            &shell,
            repo,
            &["commit", "--allow-empty", "-m", "another main commit"],
        )
        .await;
        run(&shell, repo, &["checkout", "feature"]).await;

        let result = rebase(&shell, repo, "main").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn errors_when_target_does_not_exist() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = temp_dir.path();
        let shell = ProcessShell;

        run(&shell, repo, &["init", "-b", "main"]).await;
        run(&shell, repo, &["config", "user.email", "test@example.com"]).await;
        run(&shell, repo, &["config", "user.name", "Test"]).await;
        run(
            &shell,
            repo,
            &["commit", "--allow-empty", "-m", "main commit"],
        )
        .await;

        let result = rebase(&shell, repo, "does-not-exist").await;

        assert!(result.is_err());
    }

    fn stdout(text: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: text.to_string(),
            stderr: String::new(),
            exit_code: 0,
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
        "--git-path",
        GH_STACK_MODIFY_STATE_FILE,
    ];

    #[tokio::test]
    async fn rebase_state_reads_the_git_dir_that_git_reports() {
        let git_dir = tempfile::tempdir().unwrap();
        let merge = git_dir.path().join("rebase-merge");
        std::fs::create_dir(&merge).unwrap();
        std::fs::write(merge.join("head-name"), "refs/heads/stack/layer-2\n").unwrap();
        std::fs::write(git_dir.path().join(GH_STACK_REBASE_STATE_FILE), "{}").unwrap();
        let listing = format!(
            "{}\n{}\n{}\n{}\n",
            merge.display(),
            git_dir.path().join("rebase-apply").display(),
            git_dir.path().join(GH_STACK_REBASE_STATE_FILE).display(),
            git_dir.path().join(GH_STACK_MODIFY_STATE_FILE).display()
        );
        let shell = MockShell::new().when("git", GIT_PATH_ARGS, stdout(&listing));

        let state = rebase_state(&shell, Path::new(".")).await.unwrap();

        assert_eq!(
            state,
            RebaseState {
                in_progress: true,
                gh_stack_state: true,
                gh_stack_modify_state: false,
                branch: Some("stack/layer-2".to_string()),
            }
        );
        assert!(state.is_interrupted());
    }

    #[tokio::test]
    async fn rebase_state_is_clear_when_no_rebase_files_exist() {
        let git_dir = tempfile::tempdir().unwrap();
        let listing = [
            "rebase-merge",
            "rebase-apply",
            GH_STACK_REBASE_STATE_FILE,
            GH_STACK_MODIFY_STATE_FILE,
        ]
        .map(|name| git_dir.path().join(name).display().to_string())
        .join("\n");
        let shell = MockShell::new().when("git", GIT_PATH_ARGS, stdout(&listing));

        let state = rebase_state(&shell, Path::new(".")).await.unwrap();

        assert_eq!(state, RebaseState::default());
        assert!(!state.is_interrupted());
    }

    #[tokio::test]
    async fn conflicted_files_splits_nul_separated_paths() {
        let shell = MockShell::new().when(
            "git",
            &["diff", "--name-only", "--diff-filter=U", "-z"],
            stdout("src/a.rs\0dir/with space.txt\0"),
        );

        let files = conflicted_files(&shell, Path::new(".")).await.unwrap();

        assert_eq!(files, ["src/a.rs", "dir/with space.txt"]);
    }

    #[tokio::test]
    async fn continue_never_opens_an_editor() {
        let shell = MockShell::new().when(
            "env",
            &["GIT_EDITOR=true", "git", "rebase", "--continue"],
            stdout(""),
        );

        rebase_continue(&shell, Path::new(".")).await.unwrap();

        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn stage_passes_the_path_after_a_separator() {
        let shell = MockShell::new().when("git", &["add", "--", "-odd name"], stdout(""));

        stage(&shell, Path::new("."), "-odd name").await.unwrap();
    }

    // Against a real repository stopped on a conflict (no remote, under a
    // tempdir): detection, then resolving and continuing, then aborting.

    use crate::test_fixtures::{FIXTURE_FEATURE_TEXT, conflicting_rebase_repo};

    #[tokio::test]
    async fn detects_a_real_conflict() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();

        let state = rebase_state(&ProcessShell, repo).await.unwrap();
        let files = conflicted_files(&ProcessShell, repo).await.unwrap();

        assert!(state.in_progress);
        assert!(!state.gh_stack_state);
        assert_eq!(state.branch.as_deref(), Some("feature"));
        assert_eq!(files, ["shared.txt"]);
        assert!(has_conflict_markers(repo, "shared.txt"));
        assert!(!has_conflict_markers(repo, "feature-only.txt"));
    }

    #[tokio::test]
    async fn resolving_staging_and_continuing_finishes_a_real_rebase() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();
        let shell = ProcessShell;

        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        assert!(!has_conflict_markers(repo, "shared.txt"));
        stage(&shell, repo, "shared.txt").await.unwrap();
        assert!(conflicted_files(&shell, repo).await.unwrap().is_empty());
        assert!(rebase_state(&shell, repo).await.unwrap().in_progress);

        rebase_continue(&shell, repo).await.unwrap();

        assert!(!rebase_state(&shell, repo).await.unwrap().is_interrupted());
        let head = shell
            .run(repo, "git", &["symbolic-ref", "--short", "HEAD"])
            .await
            .unwrap();
        assert_eq!(head.stdout.trim(), "feature");
        // Both feature commits were replayed on top of main.
        let log = shell
            .run(repo, "git", &["log", "--format=%s", "main..feature"])
            .await
            .unwrap();
        assert_eq!(
            log.stdout.lines().collect::<Vec<_>>(),
            ["feature: add file", "feature: change shared"]
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            "resolved\n"
        );
    }

    #[tokio::test]
    async fn continuing_with_unresolved_files_fails_and_stays_mid_rebase() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();

        let result = rebase_continue(&ProcessShell, repo).await;

        assert!(result.is_err());
        assert!(rebase_state(&ProcessShell, repo).await.unwrap().in_progress);
        assert_eq!(
            conflicted_files(&ProcessShell, repo).await.unwrap(),
            ["shared.txt"]
        );
    }

    #[tokio::test]
    async fn aborting_restores_the_branch() {
        let dir = conflicting_rebase_repo();
        let repo = dir.path();

        rebase_abort(&ProcessShell, repo).await.unwrap();

        assert!(
            !rebase_state(&ProcessShell, repo)
                .await
                .unwrap()
                .is_interrupted()
        );
        assert!(
            conflicted_files(&ProcessShell, repo)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            FIXTURE_FEATURE_TEXT
        );
    }
}
