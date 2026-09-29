use std::path::Path;

use crate::shell::{Shell, ShellError};

/// Field separator used to split `git log --format` output into columns.
/// `\x1f` (unit separator) can't appear in a commit subject or author name,
/// so splitting on it is unambiguous.
const FIELD_SEPARATOR: char = '\u{1f}';

/// Returns the one-line-per-commit `git log` output for the repository at `repo`.
#[cfg_attr(not(test), expect(dead_code, reason = "for the log panel (#27)"))]
pub async fn log(shell: &impl Shell, repo: &Path) -> Result<String, ShellError> {
    let output = shell.run(repo, "git", &["log", "--oneline"]).await?;
    Ok(output.stdout)
}

/// One commit as reported by [`log_range`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEntry {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub authored_at: String,
}

/// Lists commits reachable from `branch` but not from `base` (`git log
/// base..branch`), oldest first — matching the order `gh pr view --json
/// commits` reports — for layers with no linked pull request to hydrate
/// commits from instead.
pub async fn log_range(
    shell: &impl Shell,
    repo: &Path,
    base: &str,
    branch: &str,
) -> Result<Vec<CommitEntry>, ShellError> {
    let range = format!("{base}..{branch}");
    let format = format!("--format=%H{FIELD_SEPARATOR}%s{FIELD_SEPARATOR}%an{FIELD_SEPARATOR}%aI");

    let output = shell
        .run(repo, "git", &["log", "--reverse", &range, &format])
        .await?;

    let commits = output
        .stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(4, FIELD_SEPARATOR);
            CommitEntry {
                hash: fields.next().unwrap_or_default().to_string(),
                subject: fields.next().unwrap_or_default().to_string(),
                author: fields.next().unwrap_or_default().to_string(),
                authored_at: fields.next().unwrap_or_default().to_string(),
            }
        })
        .collect();

    Ok(commits)
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
            &["log", "--oneline"],
            Ok(ShellOutput {
                stdout: "abc1234 initial commit".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let output = log(&shell, repo.as_path()).await.unwrap();

        assert_eq!(output, "abc1234 initial commit");
    }

    async fn init_repo_with_commit(shell: &ProcessShell, repo: &Path, message: &str) {
        shell.run(repo, "git", &["init"]).await.unwrap();
        shell
            .run(repo, "git", &["config", "user.email", "test@example.com"])
            .await
            .unwrap();
        shell
            .run(repo, "git", &["config", "user.name", "Test"])
            .await
            .unwrap();
        shell
            .run(repo, "git", &["commit", "--allow-empty", "-m", message])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn returns_commit_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let shell = ProcessShell;

        init_repo_with_commit(&shell, temp_dir.path(), "initial commit").await;

        let output = log(&shell, temp_dir.path()).await.unwrap();

        assert!(output.contains("initial commit"));
    }

    #[tokio::test]
    async fn errors_for_repo_with_no_commits() {
        let temp_dir = tempfile::tempdir().unwrap();
        let shell = ProcessShell;

        shell.run(temp_dir.path(), "git", &["init"]).await.unwrap();

        let result = log(&shell, temp_dir.path()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn parses_a_delimited_commit_range_oldest_first() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &[
                "log",
                "--reverse",
                "main..layer-1",
                "--format=%H\u{1f}%s\u{1f}%an\u{1f}%aI",
            ],
            Ok(ShellOutput {
                stdout: "abc123\u{1f}feat: thing\u{1f}Jordan Evans\u{1f}2026-09-14T23:03:34Z\n"
                    .to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let commits = log_range(&shell, repo.as_path(), "main", "layer-1")
            .await
            .unwrap();

        assert_eq!(
            commits,
            vec![CommitEntry {
                hash: "abc123".to_string(),
                subject: "feat: thing".to_string(),
                author: "Jordan Evans".to_string(),
                authored_at: "2026-09-14T23:03:34Z".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn returns_no_commits_when_the_range_is_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let shell = ProcessShell;

        init_repo_with_commit(&shell, temp_dir.path(), "initial commit").await;

        let commits = log_range(&shell, temp_dir.path(), "HEAD", "HEAD")
            .await
            .unwrap();

        assert!(commits.is_empty());
    }

    #[tokio::test]
    async fn lists_commits_unique_to_a_branch_oldest_first() {
        let temp_dir = tempfile::tempdir().unwrap();
        let shell = ProcessShell;

        init_repo_with_commit(&shell, temp_dir.path(), "initial commit").await;
        let base = shell
            .run(temp_dir.path(), "git", &["rev-parse", "HEAD"])
            .await
            .unwrap()
            .stdout
            .trim()
            .to_string();
        shell
            .run(temp_dir.path(), "git", &["checkout", "-b", "layer-1"])
            .await
            .unwrap();
        init_repo_with_commit(&shell, temp_dir.path(), "first change").await;
        init_repo_with_commit(&shell, temp_dir.path(), "second change").await;

        let commits = log_range(&shell, temp_dir.path(), &base, "layer-1")
            .await
            .unwrap();

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "first change");
        assert_eq!(commits[1].subject, "second change");
        assert_eq!(commits[0].author, "Test");
    }
}
