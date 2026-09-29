use std::path::Path;

use crate::shell::{Shell, ShellError};

use super::error::{CheckFailure, Tool};
use super::version::Version;

/// Checks that `gh` is authenticated (`gh auth status` exits successfully).
/// There is no version to report, so success carries `None`.
pub async fn check(shell: &impl Shell, cwd: &Path) -> Result<Option<Version>, CheckFailure> {
    match shell.run(cwd, "gh", &["auth", "status"]).await {
        Ok(_) => Ok(None),
        Err(ShellError::BinaryNotFound(_)) => Err(CheckFailure::NotInstalled { tool: Tool::Gh }),
        Err(err) => Err(CheckFailure::Failed {
            tool: Tool::GhAuth,
            reason: err.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::MockShell;
    use crate::shell::ShellOutput;
    use std::env;

    #[tokio::test]
    async fn passes_when_authenticated() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["auth", "status"],
            Ok(ShellOutput {
                stdout: "Logged in to github.com as octocat".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        assert!(check(&shell, cwd.as_path()).await.is_ok());
    }

    #[tokio::test]
    async fn fails_when_not_authenticated() {
        let cwd = env::current_dir().unwrap();
        let command_failed = ShellError::CommandFailed {
            program: "gh".to_string(),
            output: ShellOutput {
                stdout: String::new(),
                stderr: "You are not logged into any GitHub hosts".to_string(),
                exit_code: 1,
            },
        };
        let expected_reason = command_failed.to_string();
        let shell = MockShell::new().when("gh", &["auth", "status"], Err(command_failed));

        let result = check(&shell, cwd.as_path()).await;

        assert_eq!(
            result,
            Err(CheckFailure::Failed {
                tool: Tool::GhAuth,
                reason: expected_reason,
            })
        );
    }

    #[tokio::test]
    async fn fails_when_gh_is_missing() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["auth", "status"],
            Err(ShellError::BinaryNotFound("gh".to_string())),
        );

        let result = check(&shell, cwd.as_path()).await;

        assert_eq!(result, Err(CheckFailure::NotInstalled { tool: Tool::Gh }));
    }
}
