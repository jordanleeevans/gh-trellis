//! Removes a stack via `gh stack unstack`.
//!
//! Behaviour verified against the `github/gh-stack` source (`cmd/unstack.go`,
//! same command text as the installed v0.1.1) and `gh stack unstack --help`:
//!
//! - It never prompts, so it is safe to run as a subprocess without a flag
//!   to skip confirmation.
//! - With no argument it acts on the stack containing the *currently checked
//!   out branch*, so callers must only target the current stack.
//! - `--local` only removes the stack from the local tracking file
//!   (`stack.Save`). It never creates a GitHub client, so it makes no API
//!   call, and it deletes no branch (local or remote) and touches no PR.
//! - Without `--local` it first calls the
//!   `POST repos/{owner}/{repo}/stacks/{n}/unstack` API (dissolving the
//!   stack grouping on GitHub), then removes local tracking. The command
//!   itself contains no branch deletion or PR closing. GitHub may leave PRs
//!   that are queued for merge or have auto-merge enabled stacked; the
//!   command then succeeds but keeps local tracking.

use std::path::Path;

use crate::shell::{Shell, ShellError};

/// How much of a stack to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnstackScope {
    /// `gh stack unstack --local`: local tracking only; GitHub is not contacted.
    Local,
    /// `gh stack unstack`: the stack on GitHub, then local tracking.
    LocalAndRemote,
}

/// The `gh` arguments for `scope`. Only [`UnstackScope::LocalAndRemote`] is
/// ever able to reach GitHub; the local variant always carries `--local`.
pub fn unstack_args(scope: UnstackScope) -> Vec<&'static str> {
    match scope {
        UnstackScope::Local => vec!["stack", "unstack", "--local"],
        UnstackScope::LocalAndRemote => vec!["stack", "unstack"],
    }
}

/// Runs `gh stack unstack` for the stack containing the checked-out branch.
pub async fn unstack_stack(
    shell: &impl Shell,
    repo: &Path,
    scope: UnstackScope,
) -> Result<(), ShellError> {
    shell.run(repo, "gh", &unstack_args(scope)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};

    fn ok() -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    #[test]
    fn local_scope_always_passes_local_flag() {
        assert_eq!(
            unstack_args(UnstackScope::Local),
            ["stack", "unstack", "--local"]
        );
    }

    #[test]
    fn local_scope_args_never_include_anything_remote_affecting() {
        let args = unstack_args(UnstackScope::Local);
        // Only the subcommand and `--local` are permitted.
        assert!(
            args.iter()
                .all(|a| ["stack", "unstack", "--local"].contains(a))
        );
        // No stack number (which would target GitHub directly).
        assert!(!args.iter().any(|a| a.parse::<u64>().is_ok()));
        assert!(args.contains(&"--local"));
    }

    #[test]
    fn remote_scope_omits_local_flag() {
        assert_eq!(
            unstack_args(UnstackScope::LocalAndRemote),
            ["stack", "unstack"]
        );
    }

    #[tokio::test]
    async fn local_unstack_runs_exactly_one_local_gh_call() {
        let shell = MockShell::new().when("gh", &["stack", "unstack", "--local"], ok());

        unstack_stack(&shell, Path::new("."), UnstackScope::Local)
            .await
            .unwrap();

        assert_eq!(
            shell.calls(),
            vec![(
                "gh".to_string(),
                vec![
                    "stack".to_string(),
                    "unstack".to_string(),
                    "--local".to_string()
                ]
            )]
        );
    }

    #[tokio::test]
    async fn remote_unstack_runs_gh_stack_unstack() {
        let shell = MockShell::new().when("gh", &["stack", "unstack"], ok());

        unstack_stack(&shell, Path::new("."), UnstackScope::LocalAndRemote)
            .await
            .unwrap();

        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn unstack_propagates_command_failure() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "unstack", "--local"],
            Err(ShellError::CommandFailed {
                program: "gh".to_string(),
                output: ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );

        let result = unstack_stack(&shell, Path::new("."), UnstackScope::Local).await;
        assert!(result.is_err());
    }
}
