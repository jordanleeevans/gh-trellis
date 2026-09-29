use std::path::Path;

use crate::shell::Shell;

use super::error::{CheckFailure, Tool};
use super::requirement::VersionRequirement;
use super::version::Version;

/// The minimum supported git version.
pub const MINIMUM: Version = Version::new(2, 20, 0);

const REQUIREMENT: VersionRequirement = VersionRequirement {
    tool: Tool::Git,
    program: "git",
    args: &["--version"],
    minimum: MINIMUM,
};

/// Checks that `git` is installed and meets [`MINIMUM`].
pub async fn check(shell: &impl Shell, cwd: &Path) -> Result<Option<Version>, CheckFailure> {
    REQUIREMENT.check(shell, cwd).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use std::env;

    #[test]
    fn minimum_is_2_20_0() {
        assert_eq!(MINIMUM, Version::new(2, 20, 0));
    }

    #[tokio::test]
    async fn passes_when_git_meets_minimum() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &["--version"],
            Ok(ShellOutput {
                stdout: "git version 2.43.0".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        assert!(check(&shell, cwd.as_path()).await.is_ok());
    }
}
