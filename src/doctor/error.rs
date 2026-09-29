use std::fmt;

use thiserror::Error;

use super::version::Version;

/// A dependency trellis checks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Git,
    Gh,
    GhStack,
    GhAuth,
}

impl Tool {
    /// Human-readable name used in reports.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Gh => "gh",
            Tool::GhStack => "gh stack",
            Tool::GhAuth => "gh auth",
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A single doctor check that failed. What to run about it is decided by
/// `remedy`, not stored here.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CheckFailure {
    /// The binary (or extension) could not be found at all.
    #[error("{tool} is not installed")]
    NotInstalled { tool: Tool },

    /// Installed, but older than the minimum trellis supports.
    #[error("{tool} {found} is too old (need {minimum}+)")]
    OutdatedVersion {
        tool: Tool,
        found: Version,
        minimum: Version,
    },

    /// The `--version` output couldn't be parsed.
    #[error("couldn't parse {tool}'s version from `{output}`")]
    UnparseableVersion { tool: Tool, output: String },

    /// A non-version check (e.g. `gh auth status`) failed.
    #[error("{tool} check failed: {reason}")]
    Failed { tool: Tool, reason: String },
}

impl CheckFailure {
    pub fn tool(&self) -> Tool {
        match self {
            CheckFailure::NotInstalled { tool }
            | CheckFailure::OutdatedVersion { tool, .. }
            | CheckFailure::UnparseableVersion { tool, .. }
            | CheckFailure::Failed { tool, .. } => *tool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_installed_names_the_tool() {
        let failure = CheckFailure::NotInstalled { tool: Tool::Git };

        assert_eq!(failure.to_string(), "git is not installed");
    }

    #[test]
    fn outdated_version_shows_found_and_minimum() {
        let failure = CheckFailure::OutdatedVersion {
            tool: Tool::Git,
            found: Version::new(2, 10, 0),
            minimum: Version::new(2, 20, 0),
        };

        assert_eq!(failure.to_string(), "git 2.10.0 is too old (need 2.20.0+)");
    }
}
