//! Checks the local environment has the tools trellis depends on.

mod error;
mod gh;
mod gh_auth;
mod gh_stack;
mod git;
mod remedy;
mod report;
mod requirement;
mod version;

use std::path::Path;

use crate::shell::Shell;

pub use error::{CheckFailure, Tool};
pub use remedy::Platform;
pub use report::{render, use_color};
pub use version::Version;

/// The outcome of checking one dependency.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// The check passed. Carries the version when one could be read.
    Ok(Option<Version>),
    Failed(CheckFailure),
    /// Not checked because `gh` itself is missing, so the result would only
    /// repeat that failure.
    Blocked,
}

/// One row of the dependency checklist.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub tool: Tool,
    /// The minimum version required, when the tool has one.
    pub minimum: Option<Version>,
    pub status: Status,
}

/// The result of every doctor check, in display order.
#[derive(Debug, PartialEq, Eq)]
pub struct Diagnosis(pub Vec<Finding>);

impl Diagnosis {
    pub fn is_healthy(&self) -> bool {
        self.0
            .iter()
            .all(|finding| matches!(finding.status, Status::Ok(_)))
    }
}

fn finding(
    tool: Tool,
    minimum: Option<Version>,
    result: Result<Option<Version>, CheckFailure>,
) -> Finding {
    let status = match result {
        Ok(version) => Status::Ok(version),
        Err(failure) => Status::Failed(failure),
    };
    Finding {
        tool,
        minimum,
        status,
    }
}

/// Runs every doctor check and returns all the results. It never stops at
/// the first failure, so the report can list everything that's wrong.
/// Independent checks run concurrently: they each start a process, and
/// `gh auth status` makes a network round trip.
pub async fn diagnose(shell: &impl Shell, cwd: &Path) -> Diagnosis {
    let (git, gh) = tokio::join!(git::check(shell, cwd), gh::check(shell, cwd));
    let git = finding(Tool::Git, Some(git::MINIMUM), git);
    let gh = finding(Tool::Gh, Some(gh::MINIMUM), gh);

    // Without `gh`, the auth and extension checks can only say "gh is
    // missing" again, so skip them and let the report show one clear fix.
    let gh_missing = matches!(gh.status, Status::Failed(CheckFailure::NotInstalled { .. }));
    let (stack, auth) = if gh_missing {
        let blocked = |tool, minimum| Finding {
            tool,
            minimum,
            status: Status::Blocked,
        };
        (
            blocked(Tool::GhStack, Some(gh_stack::MINIMUM)),
            blocked(Tool::GhAuth, None),
        )
    } else {
        let (stack, auth) = tokio::join!(gh_stack::check(shell, cwd), gh_auth::check(shell, cwd));
        (
            finding(Tool::GhStack, Some(gh_stack::MINIMUM), stack),
            finding(Tool::GhAuth, None, auth),
        )
    };

    Diagnosis(vec![git, gh, stack, auth])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellError, ShellOutput};
    use std::env;

    fn ok(stdout: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn not_found(program: &str) -> Result<ShellOutput, ShellError> {
        Err(ShellError::BinaryNotFound(program.to_string()))
    }

    fn healthy() -> MockShell {
        MockShell::new()
            .when("git", &["--version"], ok("git version 2.43.0"))
            .when("gh", &["--version"], ok("gh version 2.90.0 (2024-01-01)"))
            .when("gh", &["auth", "status"], ok("Logged in to github.com"))
            .when(
                "gh",
                &["extension", "list"],
                ok("NAME       REPO                     VERSION\ngh stack   github/gh-stack  v0.5.0"),
            )
    }

    async fn run(shell: &MockShell) -> Diagnosis {
        diagnose(shell, env::current_dir().unwrap().as_path()).await
    }

    fn failure(diagnosis: &Diagnosis, tool: Tool) -> Option<&CheckFailure> {
        diagnosis
            .0
            .iter()
            .find(|f| f.tool == tool)
            .and_then(|f| match &f.status {
                Status::Failed(failure) => Some(failure),
                _ => None,
            })
    }

    #[tokio::test]
    async fn healthy_machine_is_healthy_with_versions() {
        let diagnosis = run(&healthy()).await;

        assert!(diagnosis.is_healthy());
        assert_eq!(
            diagnosis.0[0].status,
            Status::Ok(Some(Version::new(2, 43, 0)))
        );
    }

    #[tokio::test]
    async fn old_git_is_the_only_failure() {
        let shell = MockShell::new()
            .when("git", &["--version"], ok("git version 2.10.0"))
            .when("gh", &["--version"], ok("gh version 2.90.0 (2024-01-01)"))
            .when("gh", &["auth", "status"], ok("Logged in to github.com"))
            .when(
                "gh",
                &["extension", "list"],
                ok("NAME  REPO  VERSION\ngh stack  github/gh-stack  v0.5.0"),
            );

        let diagnosis = run(&shell).await;

        assert!(matches!(
            failure(&diagnosis, Tool::Git),
            Some(CheckFailure::OutdatedVersion { .. })
        ));
        assert_eq!(
            diagnosis
                .0
                .iter()
                .filter(|f| !matches!(f.status, Status::Ok(_)))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn missing_gh_stack_is_reported() {
        let shell = MockShell::new()
            .when("git", &["--version"], ok("git version 2.43.0"))
            .when("gh", &["--version"], ok("gh version 2.90.0 (2024-01-01)"))
            .when("gh", &["auth", "status"], ok("Logged in"))
            .when("gh", &["extension", "list"], ok("NAME  REPO  VERSION"));

        let diagnosis = run(&shell).await;

        assert_eq!(
            failure(&diagnosis, Tool::GhStack),
            Some(&CheckFailure::NotInstalled {
                tool: Tool::GhStack
            })
        );
    }

    #[tokio::test]
    async fn every_failure_is_collected_not_just_the_first() {
        let shell = MockShell::new()
            .when("git", &["--version"], ok("git version 2.10.0"))
            .when("gh", &["--version"], ok("gh version 2.1.0"))
            .when(
                "gh",
                &["auth", "status"],
                Err(ShellError::CommandFailed {
                    program: "gh".to_string(),
                    output: ShellOutput {
                        stdout: String::new(),
                        stderr: "not logged in".to_string(),
                        exit_code: 1,
                    },
                }),
            )
            .when("gh", &["extension", "list"], ok("NAME  REPO  VERSION"));

        let diagnosis = run(&shell).await;

        assert!(failure(&diagnosis, Tool::Git).is_some());
        assert!(failure(&diagnosis, Tool::Gh).is_some());
        assert!(failure(&diagnosis, Tool::GhStack).is_some());
        assert!(failure(&diagnosis, Tool::GhAuth).is_some());
    }

    #[tokio::test]
    async fn missing_gh_blocks_dependent_checks_instead_of_repeating_itself() {
        let shell = MockShell::new()
            .when("git", &["--version"], not_found("git"))
            .when("gh", &["--version"], not_found("gh"));

        let diagnosis = run(&shell).await;

        assert_eq!(
            failure(&diagnosis, Tool::Git),
            Some(&CheckFailure::NotInstalled { tool: Tool::Git })
        );
        assert_eq!(
            failure(&diagnosis, Tool::Gh),
            Some(&CheckFailure::NotInstalled { tool: Tool::Gh })
        );
        assert_eq!(diagnosis.0[2].status, Status::Blocked);
        assert_eq!(diagnosis.0[3].status, Status::Blocked);
        assert_eq!(shell.calls().len(), 2);
    }
}
