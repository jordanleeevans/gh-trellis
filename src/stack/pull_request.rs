use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::shell::{Shell, ShellError};

/// A pull request linked to a [`Layer`](super::Layer).
///
/// `number`, `url`, and `state` come straight from `gh stack view --json`'s
/// nested `pr` object. `title`, `is_draft`, `checks_status`, and
/// `review_decision` aren't present there — they're only available from
/// `gh pr view --json` (as `title`, `isDraft`, a derived checks summary, and
/// `reviewDecision`) and are filled in by a separate hydration step, so they
/// stay `None` until that runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRef {
    pub number: u64,
    pub url: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_decision: Option<String>,
}

/// Raw shape of `gh pr view --json number,url,state,title,isDraft,reviewDecision`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrView {
    number: u64,
    url: String,
    state: String,
    title: String,
    is_draft: bool,
    review_decision: String,
}

impl From<GhPrView> for PullRequestRef {
    fn from(view: GhPrView) -> Self {
        PullRequestRef {
            number: view.number,
            url: view.url,
            state: view.state,
            title: Some(view.title),
            is_draft: Some(view.is_draft),
            checks_status: None,
            review_decision: Some(view.review_decision),
        }
    }
}

/// Looks up the pull request for `branch` via `gh pr view --json`.
///
/// Returns `Ok(None)` when the branch has no linked pull request — `gh pr
/// view` exits non-zero in that case, which isn't a real error for callers
/// enumerating stacks that may have unsubmitted layers.
pub async fn hydrate_pull_request(
    shell: &impl Shell,
    repo: &Path,
    branch: &str,
) -> Result<Option<PullRequestRef>, ShellError> {
    let result = shell
        .run(
            repo,
            "gh",
            &[
                "pr",
                "view",
                branch,
                "--json",
                "number,url,state,title,isDraft,reviewDecision",
            ],
        )
        .await;

    let output = match result {
        Ok(output) => output,
        Err(ShellError::CommandFailed { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };

    let view: GhPrView = serde_json::from_str(&output.stdout)
        .map_err(|error| ShellError::UnexpectedOutput(error.to_string()))?;

    Ok(Some(view.into()))
}

/// How many of a branch's most recent pull requests to consider, as
/// `gh pr view <branch>` does.
const PRS_PER_BRANCH: usize = 30;

/// The fields [`PullRequestRef`] needs, as GraphQL returns them.
const PR_FIELDS: &str = "number url state title isDraft reviewDecision isCrossRepository";

/// The `gh api graphql` arguments that fetch every branch's pull requests in
/// one request: one aliased `pullRequests(headRefName:)` connection per
/// branch (`b0`, `b1`, ...), with the branch names passed as variables.
/// `{owner}` and `{repo}` are filled in by gh from the current repository.
pub fn batch_query_args(branches: &[String]) -> Vec<String> {
    let variables: Vec<String> = (0..branches.len())
        .map(|index| format!(", $b{index}: String!"))
        .collect();
    let connections: Vec<String> = (0..branches.len())
        .map(|index| {
            format!(
                "b{index}: pullRequests(headRefName: $b{index}, first: {PRS_PER_BRANCH}, \
                 orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} }}"
            )
        })
        .collect();
    let query = format!(
        "query($owner: String!, $name: String!{}) {{ repository(owner: $owner, name: $name) {{ {} }} }}",
        variables.concat(),
        connections.join(" ")
    );

    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-F".to_string(),
        "owner={owner}".to_string(),
        "-F".to_string(),
        "name={repo}".to_string(),
    ];
    for (index, branch) in branches.iter().enumerate() {
        // -f (raw string), not -F, so a branch name is never type-converted.
        args.push("-f".to_string());
        args.push(format!("b{index}={branch}"));
    }
    args.push("-f".to_string());
    args.push(format!("query={query}"));
    args
}

#[derive(Debug, Deserialize)]
struct BatchResponse {
    data: BatchData,
}

#[derive(Debug, Deserialize)]
struct BatchData {
    repository: HashMap<String, Connection>,
}

#[derive(Debug, Deserialize)]
struct Connection {
    nodes: Vec<GraphQlPr>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPr {
    number: u64,
    url: String,
    state: String,
    title: String,
    is_draft: bool,
    review_decision: Option<String>,
    is_cross_repository: bool,
}

impl From<GraphQlPr> for PullRequestRef {
    fn from(pr: GraphQlPr) -> Self {
        PullRequestRef {
            number: pr.number,
            url: pr.url,
            state: pr.state,
            title: Some(pr.title),
            is_draft: Some(pr.is_draft),
            checks_status: None,
            // `gh pr view --json reviewDecision` prints "" where GraphQL has null.
            review_decision: Some(pr.review_decision.unwrap_or_default()),
        }
    }
}

/// Picks the pull request `gh pr view <branch>` would show from a branch's
/// most recent pull requests (newest first): pull requests from forks are
/// skipped, and an open one wins over closed or merged ones. Mirrors
/// `findForBranch` in the GitHub CLI's `pkg/cmd/pr/shared/finder.go`.
fn pick_for_branch(newest_first: Vec<GraphQlPr>) -> Option<GraphQlPr> {
    let same_repo = newest_first
        .into_iter()
        .filter(|pr| !pr.is_cross_repository);
    let (open, rest): (Vec<_>, Vec<_>) = same_repo.partition(|pr| pr.state == "OPEN");
    open.into_iter().chain(rest).next()
}

/// Looks up the pull request for every branch in `branches` with a single
/// `gh api graphql` request, instead of one `gh pr view` per branch (each a
/// separate process and round trip, which made refreshing a stack take
/// seconds). Branches with no pull request are absent from the map.
///
/// If the batched request fails (e.g. no GitHub remote, or offline), it
/// falls back to [`hydrate_pull_request`] per branch, so the result is never
/// worse than looking each branch up on its own.
pub async fn hydrate_pull_requests(
    shell: &impl Shell,
    repo: &Path,
    branches: &[String],
) -> Result<HashMap<String, PullRequestRef>, ShellError> {
    if branches.is_empty() {
        return Ok(HashMap::new());
    }

    let args = batch_query_args(branches);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let batched = match shell.run(repo, "gh", &arg_refs).await {
        Ok(output) => serde_json::from_str::<BatchResponse>(&output.stdout).ok(),
        Err(_) => None,
    };

    let Some(mut response) = batched else {
        let mut found = HashMap::new();
        for branch in branches {
            if let Some(pr) = hydrate_pull_request(shell, repo, branch).await? {
                found.insert(branch.clone(), pr);
            }
        }
        return Ok(found);
    };

    Ok(branches
        .iter()
        .enumerate()
        .filter_map(|(index, branch)| {
            let nodes = response.data.repository.remove(&format!("b{index}"))?.nodes;
            pick_for_branch(nodes).map(|pr| (branch.clone(), pr.into()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphql_pr(number: u64, state: &str, cross_repo: bool) -> GraphQlPr {
        GraphQlPr {
            number,
            url: format!("u/{number}"),
            state: state.to_string(),
            title: "t".to_string(),
            is_draft: false,
            review_decision: None,
            is_cross_repository: cross_repo,
        }
    }

    #[test]
    fn picks_an_open_pr_over_a_newer_closed_or_merged_one() {
        let newest_first = vec![
            graphql_pr(9, "MERGED", false),
            graphql_pr(5, "OPEN", false),
            graphql_pr(3, "CLOSED", false),
        ];
        assert_eq!(pick_for_branch(newest_first).unwrap().number, 5);
    }

    #[test]
    fn picks_the_newest_when_none_is_open() {
        let newest_first = vec![
            graphql_pr(9, "MERGED", false),
            graphql_pr(3, "CLOSED", false),
        ];
        assert_eq!(pick_for_branch(newest_first).unwrap().number, 9);
    }

    #[test]
    fn skips_pull_requests_from_forks() {
        let newest_first = vec![graphql_pr(9, "OPEN", true), graphql_pr(3, "MERGED", false)];
        assert_eq!(pick_for_branch(newest_first).unwrap().number, 3);
        assert!(pick_for_branch(vec![graphql_pr(9, "OPEN", true)]).is_none());
        assert!(pick_for_branch(Vec::new()).is_none());
    }

    #[test]
    fn batch_query_passes_branches_as_raw_string_variables() {
        let args = batch_query_args(&["feat/a".to_string(), "123".to_string()]);
        assert_eq!(
            &args[..6],
            ["api", "graphql", "-F", "owner={owner}", "-F", "name={repo}"]
        );
        assert_eq!(&args[6..10], ["-f", "b0=feat/a", "-f", "b1=123"]);
        let query = args.last().unwrap();
        assert!(query.starts_with(
            "query=query($owner: String!, $name: String!, $b0: String!, $b1: String!)"
        ));
        assert!(query.contains("b1: pullRequests(headRefName: $b1, first: 30,"));
    }
    use crate::shell::{MockShell, ShellOutput};

    #[tokio::test]
    async fn hydrates_a_pull_request_from_gh_pr_view() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "view",
                "layer-1",
                "--json",
                "number,url,state,title,isDraft,reviewDecision",
            ],
            Ok(ShellOutput {
                stdout: r#"{"number":44,"url":"https://github.com/o/r/pull/44","state":"OPEN","title":"stack demo/layer 1","isDraft":true,"reviewDecision":""}"#.to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let pr = hydrate_pull_request(&shell, repo.as_path(), "layer-1")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(pr.number, 44);
        assert_eq!(pr.title.as_deref(), Some("stack demo/layer 1"));
        assert_eq!(pr.is_draft, Some(true));
        assert_eq!(pr.review_decision.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn returns_none_when_the_branch_has_no_pull_request() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "view",
                "unsubmitted",
                "--json",
                "number,url,state,title,isDraft,reviewDecision",
            ],
            Err(ShellError::CommandFailed {
                program: "gh".to_string(),
                output: ShellOutput {
                    stdout: String::new(),
                    stderr: "no pull requests found".to_string(),
                    exit_code: 1,
                },
            }),
        );

        let pr = hydrate_pull_request(&shell, repo.as_path(), "unsubmitted")
            .await
            .unwrap();

        assert_eq!(pr, None);
    }

    #[tokio::test]
    async fn propagates_other_shell_errors() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "gh",
            &[
                "pr",
                "view",
                "layer-1",
                "--json",
                "number,url,state,title,isDraft,reviewDecision",
            ],
            Err(ShellError::BinaryNotFound("gh".to_string())),
        );

        let result = hydrate_pull_request(&shell, repo.as_path(), "layer-1").await;

        assert!(matches!(result, Err(ShellError::BinaryNotFound(_))));
    }

    /// A `PullRequestRef` after hydration, in *our* wire shape (this
    /// struct's own field names — not raw `gh pr view --json`, which uses
    /// `isDraft`/`reviewDecision`). Values are the real ones captured for
    /// PR #44; `checks_status` is omitted since no hydration step computes
    /// it from `statusCheckRollup` yet.
    const HYDRATED_FIXTURE: &str = include_str!("fixtures/pr_ref_hydrated.json");

    #[test]
    fn parses_a_hydrated_pull_request_ref() {
        let pr: PullRequestRef = serde_json::from_str(HYDRATED_FIXTURE).unwrap();

        assert_eq!(pr.number, 44);
        assert_eq!(pr.url, "https://github.com/jordanleeevans/trellis/pull/44");
        assert_eq!(pr.state, "OPEN");
        assert_eq!(pr.title.as_deref(), Some("stack demo/layer 1"));
        assert_eq!(pr.is_draft, Some(true));
        assert_eq!(pr.checks_status, None);
        assert_eq!(pr.review_decision.as_deref(), Some(""));
    }

    #[test]
    fn round_trips_a_hydrated_pull_request_ref_without_loss() {
        let pr: PullRequestRef = serde_json::from_str(HYDRATED_FIXTURE).unwrap();

        let reserialized: serde_json::Value = serde_json::to_value(&pr).unwrap();
        let original: serde_json::Value = serde_json::from_str(HYDRATED_FIXTURE).unwrap();

        assert_eq!(reserialized, original);
    }

    #[test]
    fn parses_the_thin_ref_from_stack_view_with_no_optional_fields() {
        let json = r#"{"number":44,"url":"https://github.com/jordanleeevans/trellis/pull/44","state":"OPEN"}"#;

        let pr: PullRequestRef = serde_json::from_str(json).unwrap();

        assert_eq!(pr.number, 44);
        assert_eq!(pr.title, None);
        assert_eq!(pr.is_draft, None);
        assert_eq!(pr.checks_status, None);
        assert_eq!(pr.review_decision, None);
    }
}
