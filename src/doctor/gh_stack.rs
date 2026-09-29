use std::path::Path;

use crate::shell::{Shell, ShellError};

use super::error::{CheckFailure, Tool};
use super::version::Version;

/// The minimum supported `gh stack` extension version.
pub const MINIMUM: Version = Version::new(0, 1, 0);

/// Checks that the `gh stack` extension is installed (and, when a version
/// can be determined, meets [`MINIMUM`]).
pub async fn check(shell: &impl Shell, cwd: &Path) -> Result<Option<Version>, CheckFailure> {
    let output = match shell.run(cwd, "gh", &["extension", "list"]).await {
        Ok(output) => output,
        Err(ShellError::BinaryNotFound(_)) => {
            return Err(CheckFailure::NotInstalled { tool: Tool::Gh });
        }
        Err(err) => {
            return Err(CheckFailure::Failed {
                tool: Tool::GhStack,
                reason: err.to_string(),
            });
        }
    };

    let stack_line = output
        .stdout
        .lines()
        .find(|line| line.to_lowercase().contains("stack"));

    let Some(stack_line) = stack_line else {
        return Err(CheckFailure::NotInstalled {
            tool: Tool::GhStack,
        });
    };

    // `gh extension list` doesn't reliably print a parseable version for
    // every extension; only enforce the minimum when one is present.
    match Version::parse(stack_line) {
        Ok(found) if found < MINIMUM => Err(CheckFailure::OutdatedVersion {
            tool: Tool::GhStack,
            found,
            minimum: MINIMUM,
        }),
        Ok(found) => Ok(Some(found)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use std::env;

    fn stdout(text: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: text.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    #[tokio::test]
    async fn passes_when_extension_is_listed() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["extension", "list"],
            stdout("NAME       REPO                          VERSION\ngh stack   timothyandrew/gh-stack       v0.5.0"),
        );

        let result = check(&shell, cwd.as_path()).await;

        assert_eq!(result, Ok(Some(Version::new(0, 5, 0))));
    }

    #[tokio::test]
    async fn fails_when_extension_is_not_listed() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["extension", "list"],
            stdout("NAME       REPO                          VERSION"),
        );

        let result = check(&shell, cwd.as_path()).await;

        assert_eq!(
            result,
            Err(CheckFailure::NotInstalled {
                tool: Tool::GhStack,
            })
        );
    }

    #[tokio::test]
    async fn fails_when_gh_is_missing() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["extension", "list"],
            Err(ShellError::BinaryNotFound("gh".to_string())),
        );

        let result = check(&shell, cwd.as_path()).await;

        assert_eq!(result, Err(CheckFailure::NotInstalled { tool: Tool::Gh }));
    }
}
