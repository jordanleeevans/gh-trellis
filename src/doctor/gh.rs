use std::path::Path;

use crate::shell::Shell;

use super::error::{CheckFailure, Tool};
use super::requirement::VersionRequirement;
use super::version::Version;

/// The minimum supported GitHub CLI (`gh`) version.
pub const MINIMUM: Version = Version::new(2, 90, 0);

const REQUIREMENT: VersionRequirement = VersionRequirement {
    tool: Tool::Gh,
    program: "gh",
    args: &["--version"],
    minimum: MINIMUM,
};

/// Checks that `gh` is installed and meets [`MINIMUM`].
pub async fn check(shell: &impl Shell, cwd: &Path) -> Result<Option<Version>, CheckFailure> {
    REQUIREMENT.check(shell, cwd).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use std::env;

    #[test]
    fn minimum_is_2_90_0() {
        assert_eq!(MINIMUM, Version::new(2, 90, 0));
    }

    #[tokio::test]
    async fn passes_when_gh_meets_minimum() {
        let cwd = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &["--version"],
            Ok(ShellOutput {
                stdout: "gh version 2.90.0 (2024-01-01)".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        assert!(check(&shell, cwd.as_path()).await.is_ok());
    }
}
