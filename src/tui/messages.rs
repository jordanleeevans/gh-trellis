//! User-facing wording for failures, kept in one place so every action
//! reports shell errors the same way.

use crate::shell::ShellError;

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
