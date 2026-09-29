use std::path::Path;

use thiserror::Error;

use crate::shell::{Shell, ShellError};

use super::layer::Layer;
use super::local::{LocalStackError, read_local_stacks};
use super::pull_request::hydrate_pull_requests;

/// Errors that can occur while enumerating locally tracked stacks.
#[derive(Debug, Error)]
pub enum StackSummaryError {
    #[error(transparent)]
    LocalStack(#[from] LocalStackError),

    #[error(transparent)]
    Shell(#[from] ShellError),
}

/// A locally tracked stack, summarized for display in the entry-point panel.
#[derive(Debug, Clone, PartialEq)]
pub struct StackSummary {
    /// A human-readable label for the stack: its bottom branch, or
    /// `bottom → top` when it has more than one layer.
    pub label: String,
    pub trunk: String,
    pub layers: Vec<Layer>,
    /// Whether the currently checked-out branch belongs to this stack.
    pub is_current: bool,
}

/// Enumerates every stack tracked locally in `repo`, hydrating each layer's
/// pull request status from `gh pr view`.
///
/// `gh stack` has no command to list every local stack at once, so this
/// reads `.git/gh-stack` directly (see [`super::local`]) and looks up each
/// branch's pull request individually.
pub async fn list_stacks(
    shell: &impl Shell,
    repo: &Path,
) -> Result<Vec<StackSummary>, StackSummaryError> {
    let local_stacks = read_local_stacks(repo)?;

    let current_branch = shell
        .run(repo, "git", &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .map(|output| output.stdout.trim().to_string())
        .unwrap_or_default();

    // Every branch across every stack, looked up in one request.
    let mut branch_names: Vec<String> = Vec::new();
    for branch in local_stacks.iter().flat_map(|stack| &stack.branches) {
        if !branch_names.contains(&branch.branch) {
            branch_names.push(branch.branch.clone());
        }
    }
    let pull_requests = hydrate_pull_requests(shell, repo, &branch_names).await?;

    let mut summaries = Vec::with_capacity(local_stacks.len());

    for local_stack in local_stacks {
        let mut layers = Vec::with_capacity(local_stack.branches.len());
        let mut is_current = false;

        for (position, branch) in local_stack.branches.iter().enumerate() {
            let pull_request = pull_requests.get(&branch.branch).cloned();
            let branch_is_current = branch.branch == current_branch;
            is_current |= branch_is_current;

            let is_merged = pull_request.as_ref().is_some_and(|pr| pr.state == "MERGED");

            layers.push(Layer {
                branch: branch.branch.clone(),
                head: None,
                base: branch.base.clone(),
                is_current: branch_is_current,
                is_merged,
                is_queued: false,
                needs_rebase: false,
                pull_request,
                commits: Vec::new(),
                position,
            });
        }

        let label = match (layers.first(), layers.last()) {
            (Some(bottom), Some(top)) if bottom.branch != top.branch => {
                format!("{} → {}", bottom.branch, top.branch)
            }
            (Some(bottom), _) => bottom.branch.clone(),
            (None, _) => String::from("(empty stack)"),
        };

        summaries.push(StackSummary {
            label,
            trunk: local_stack.trunk.branch,
            layers,
            is_current,
        });
    }

    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};

    fn ok(stdout: &str) -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn no_pr(branch: &str) -> Result<ShellOutput, ShellError> {
        let _ = branch;
        Err(ShellError::CommandFailed {
            program: "gh".to_string(),
            output: ShellOutput {
                stdout: String::new(),
                stderr: "no pull requests found".to_string(),
                exit_code: 1,
            },
        })
    }

    /// Registers the single batched PR lookup for `branches`, answered with
    /// `nodes[i]` (a JSON array of pull requests) for branch `b{i}`.
    fn when_batched(shell: MockShell, branches: &[&str], nodes: &[&str]) -> MockShell {
        let names: Vec<String> = branches.iter().map(|b| b.to_string()).collect();
        let args = crate::stack::pull_request::batch_query_args(&names);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let connections: Vec<String> = nodes
            .iter()
            .enumerate()
            .map(|(i, nodes)| format!(r#""b{i}":{{"nodes":{nodes}}}"#))
            .collect();
        let body = format!(
            r#"{{"data":{{"repository":{{{}}}}}}}"#,
            connections.join(",")
        );
        shell.when("gh", &args, ok(&body))
    }

    const OPEN_PR_1: &str = r#"[{"number":1,"url":"u","state":"OPEN","title":"t","isDraft":false,"reviewDecision":null,"isCrossRepository":false}]"#;

    fn write_local_stacks(repo: &Path, json: &str) {
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("gh-stack"), json).unwrap();
    }

    #[tokio::test]
    async fn returns_empty_when_there_are_no_local_stacks() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        let shell =
            MockShell::new().when("git", &["rev-parse", "--abbrev-ref", "HEAD"], ok("main"));

        let summaries = list_stacks(&shell, temp_dir.path()).await.unwrap();

        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn summarizes_a_stack_and_marks_it_current() {
        let temp_dir = tempfile::tempdir().unwrap();
        write_local_stacks(
            temp_dir.path(),
            r#"{
                "stacks": [
                    {
                        "trunk": { "branch": "main" },
                        "branches": [
                            { "branch": "layer-1", "base": "main" },
                            { "branch": "layer-2", "base": "layer-1" }
                        ]
                    }
                ]
            }"#,
        );

        let shell =
            MockShell::new().when("git", &["rev-parse", "--abbrev-ref", "HEAD"], ok("layer-2"));
        let shell = when_batched(shell, &["layer-1", "layer-2"], &[OPEN_PR_1, "[]"]);

        let summaries = list_stacks(&shell, temp_dir.path()).await.unwrap();

        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.label, "layer-1 → layer-2");
        assert_eq!(summary.trunk, "main");
        assert_eq!(summary.layers.len(), 2);
        assert!(summary.is_current);
        assert!(!summary.layers[0].is_current);
        assert!(summary.layers[1].is_current);

        // layer-1's pull request was hydrated; layer-2 has none yet.
        let pr = summary.layers[0].pull_request.as_ref().unwrap();
        assert_eq!(pr.number, 1);
        assert_eq!(pr.review_decision.as_deref(), Some(""));
        assert!(summary.layers[1].pull_request.is_none());

        // One git call and a single gh request for both branches.
        assert_eq!(shell.calls().len(), 2);
    }

    /// Without a usable batched response (offline, no GitHub remote), each
    /// branch is looked up on its own, as before batching.
    #[tokio::test]
    async fn falls_back_to_a_lookup_per_branch_when_the_batch_fails() {
        let temp_dir = tempfile::tempdir().unwrap();
        write_local_stacks(
            temp_dir.path(),
            r#"{
                "stacks": [
                    {
                        "trunk": { "branch": "main" },
                        "branches": [ { "branch": "layer-1", "base": "main" } ]
                    }
                ]
            }"#,
        );
        let names = vec!["layer-1".to_string()];
        let batch = crate::stack::pull_request::batch_query_args(&names);
        let batch: Vec<&str> = batch.iter().map(String::as_str).collect();
        let shell = MockShell::new()
            .when("git", &["rev-parse", "--abbrev-ref", "HEAD"], ok("main"))
            .when("gh", &batch, no_pr("graphql"))
            .when(
                "gh",
                &[
                    "pr",
                    "view",
                    "layer-1",
                    "--json",
                    "number,url,state,title,isDraft,reviewDecision",
                ],
                ok(r#"{"number":7,"url":"u","state":"OPEN","title":"t","isDraft":false,"reviewDecision":""}"#),
            );

        let summaries = list_stacks(&shell, temp_dir.path()).await.unwrap();

        assert_eq!(
            summaries[0].layers[0].pull_request.as_ref().unwrap().number,
            7
        );
    }

    #[tokio::test]
    async fn a_stack_without_the_current_branch_is_not_current() {
        let temp_dir = tempfile::tempdir().unwrap();
        write_local_stacks(
            temp_dir.path(),
            r#"{
                "stacks": [
                    {
                        "trunk": { "branch": "main" },
                        "branches": [ { "branch": "layer-1", "base": "main" } ]
                    }
                ]
            }"#,
        );

        let shell = MockShell::new().when(
            "git",
            &["rev-parse", "--abbrev-ref", "HEAD"],
            ok("unrelated-branch"),
        );
        let shell = when_batched(shell, &["layer-1"], &["[]"]);

        let summaries = list_stacks(&shell, temp_dir.path()).await.unwrap();

        assert!(!summaries[0].is_current);
        assert_eq!(summaries[0].label, "layer-1");
    }

    #[tokio::test]
    async fn propagates_timeout_errors_while_hydrating_pull_requests() {
        let temp_dir = tempfile::tempdir().unwrap();
        write_local_stacks(
            temp_dir.path(),
            r#"{
                "stacks": [
                    {
                        "trunk": { "branch": "main" },
                        "branches": [ { "branch": "layer-1", "base": "main" } ]
                    }
                ]
            }"#,
        );

        let names = vec!["layer-1".to_string()];
        let batch = crate::stack::pull_request::batch_query_args(&names);
        let batch: Vec<&str> = batch.iter().map(String::as_str).collect();
        let timeout = || {
            Err(ShellError::Timeout {
                program: "gh".to_string(),
                timeout: std::time::Duration::from_secs(1),
            })
        };
        // The batch times out, and so does the per-branch fallback.
        let shell = MockShell::new()
            .when("git", &["rev-parse", "--abbrev-ref", "HEAD"], ok("main"))
            .when("gh", &batch, timeout())
            .when(
                "gh",
                &[
                    "pr",
                    "view",
                    "layer-1",
                    "--json",
                    "number,url,state,title,isDraft,reviewDecision",
                ],
                Err(ShellError::Timeout {
                    program: "gh".to_string(),
                    timeout: std::time::Duration::from_secs(1),
                }),
            );

        let result = list_stacks(&shell, temp_dir.path()).await;

        assert!(matches!(
            result,
            Err(StackSummaryError::Shell(ShellError::Timeout { .. }))
        ));
    }
}
