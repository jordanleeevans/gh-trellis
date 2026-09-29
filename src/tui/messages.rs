//! User-facing wording for failures, kept in one place so every action
//! reports shell errors the same way.

use crate::shell::ShellError;
use crate::stack::{MergeFailure, MergeRefusal};

pub(crate) fn friendly_stack_refresh_error(error: &str) -> String {
    let normalized = error.to_lowercase();

    if normalized.contains("timed out") {
        return "request timed out while refreshing stacks; check network/auth and retry"
            .to_string();
    }
    if normalized.contains("rate limit") {
        return "GitHub API rate limit reached; retry later".to_string();
    }
    if normalized.contains("not logged in") || normalized.contains("authentication") {
        return "GitHub authentication required; run `gh auth login`".to_string();
    }
    if normalized.contains("network") || normalized.contains("could not resolve host") {
        return "network failure while refreshing stacks".to_string();
    }

    error.to_string()
}

pub(crate) fn friendly_shell_error(context: &str, error: &ShellError) -> String {
    match error {
        ShellError::Timeout { .. } => {
            format!("{context}: request timed out; check network/auth and retry")
        }
        ShellError::BinaryNotFound(program) => {
            format!("{context}: required binary `{program}` not found")
        }
        ShellError::CommandFailed { program, output } if program == "gh" => {
            let stderr = output.stderr.to_lowercase();
            if stderr.contains("rate limit") {
                format!("{context}: GitHub API rate limit reached; retry later")
            } else if stderr.contains("not logged in")
                || stderr.contains("authentication")
                || stderr.contains("401")
            {
                format!("{context}: GitHub authentication required; run `gh auth login`")
            } else if stderr.contains("network")
                || stderr.contains("timed out")
                || stderr.contains("could not resolve host")
            {
                format!("{context}: network failure while calling gh")
            } else {
                format!("{context}: {}", output.stderr.trim())
            }
        }
        ShellError::CommandFailed { output, .. } => format!("{context}: {}", output.stderr.trim()),
        _ => format!("{context}: {error}"),
    }
}

/// Why trellis won't offer to merge a stack. Mirrors gh-stack's own
/// wording where the refusal is gh-stack's (see `stack::merge`).
pub(crate) fn merge_refusal_message(refusal: &MergeRefusal) -> String {
    match refusal {
        MergeRefusal::Empty => "selected stack has no layers to merge".to_string(),
        MergeRefusal::AlreadyMerged => "this stack is already fully merged".to_string(),
        MergeRefusal::NothingToMerge => {
            "nothing to merge: no layer has an open pull request (submit the stack first)"
                .to_string()
        }
        MergeRefusal::Blocked {
            number,
            branch,
            reason,
        } => format!(
            "cannot merge the whole stack: #{number} ({branch}) is {}. gh stack merge only merges open, non-draft PRs",
            reason.describe()
        ),
        MergeRefusal::UnsubmittedBelow { branch } => format!(
            "cannot merge: {branch} has no pull request but sits below PRs that would merge; submit the stack first"
        ),
    }
}

/// A failed `gh stack merge`, as the user should read it.
pub(crate) fn merge_failure_message(failure: &MergeFailure) -> String {
    match failure {
        MergeFailure::Rejected { message } => {
            format!("merge failed, nothing was merged (stack merges are atomic): {message}")
        }
        MergeFailure::StillInProgress => {
            "merge started but hadn't finished when trellis stopped waiting; it may still complete. Check the pull requests on GitHub before retrying".to_string()
        }
        MergeFailure::Refused { message } => format!("merge stack: {message}"),
        MergeFailure::Shell(error) => friendly_shell_error("merge stack", error),
    }
}
