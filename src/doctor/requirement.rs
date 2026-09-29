use std::path::Path;

use crate::shell::{Shell, ShellError};

use super::error::{CheckFailure, Tool};
use super::version::Version;

/// A single version-gated dependency check: run a command, parse a version
/// out of its output, and compare it against a minimum.
pub struct VersionRequirement {
    pub tool: Tool,
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub minimum: Version,
}

impl VersionRequirement {
    /// Runs the check, returning the version found if `program` is installed
    /// and meets `minimum`.
    pub async fn check(
        &self,
        shell: &impl Shell,
        cwd: &Path,
    ) -> Result<Option<Version>, CheckFailure> {
        let output = match shell.run(cwd, self.program, self.args).await {
            Ok(output) => output,
            Err(ShellError::BinaryNotFound(_)) => {
                return Err(CheckFailure::NotInstalled { tool: self.tool });
            }
            Err(err) => {
                return Err(CheckFailure::Failed {
                    tool: self.tool,
                    reason: err.to_string(),
                });
            }
        };

        let found =
            Version::parse(&output.stdout).map_err(|_| CheckFailure::UnparseableVersion {
                tool: self.tool,
                output: output.stdout.clone(),
            })?;

        if found < self.minimum {
            return Err(CheckFailure::OutdatedVersion {
                tool: self.tool,
                found,
                minimum: self.minimum,
            });
        }

        Ok(Some(found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use std::env;

    fn requirement() -> VersionRequirement {
        VersionRequirement {
            tool: Tool::Git,
            program: "git",
            args: &["--version"],
            minimum: Version::new(2, 20, 0),
        }
    }

    fn stdout(text: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: text.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    #[tokio::test]
    async fn passes_when_version_meets_minimum() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when("git", &["--version"], stdout("git version 2.43.0"));

        let result = requirement().check(&shell, cwd.as_path()).await;

        assert_eq!(result, Ok(Some(Version::new(2, 43, 0))));
    }

    #[tokio::test]
    async fn fails_when_version_is_below_minimum() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when("git", &["--version"], stdout("git version 2.10.0"));

        let result = requirement().check(&shell, cwd.as_path()).await;

        assert_eq!(
            result,
            Err(CheckFailure::OutdatedVersion {
                tool: Tool::Git,
                found: Version::new(2, 10, 0),
                minimum: Version::new(2, 20, 0),
            })
        );
    }

    #[tokio::test]
    async fn fails_when_binary_is_missing() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &["--version"],
            Err(ShellError::BinaryNotFound("git".to_string())),
        );

        let result = requirement().check(&shell, cwd.as_path()).await;

        assert_eq!(result, Err(CheckFailure::NotInstalled { tool: Tool::Git }));
    }

    #[tokio::test]
    async fn fails_when_version_cannot_be_parsed() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when("git", &["--version"], stdout("not a version"));

        let result = requirement().check(&shell, cwd.as_path()).await;

        assert_eq!(
            result,
            Err(CheckFailure::UnparseableVersion {
                tool: Tool::Git,
                output: "not a version".to_string(),
            })
        );
    }
}
