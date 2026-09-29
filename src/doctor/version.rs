//! Version parsing and comparison shared by all doctor checks.

use std::fmt;

/// A `major.minor.patch` version extracted from a tool's `--version` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses the first `major.minor.patch` token found in `input`.
    ///
    /// Tool `--version` output varies in its surrounding text (e.g.
    /// `"git version 2.43.0"` vs `"gh version 2.40.1 (2023-12-13)"`), so
    /// this scans whitespace-separated tokens rather than expecting a fixed
    /// prefix.
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        input
            .split_whitespace()
            .find_map(Self::parse_token)
            .ok_or("no version number found")
    }

    fn parse_token(token: &str) -> Option<Self> {
        // Extensions list tags like `v0.5.0`.
        let mut parts = token.strip_prefix('v').unwrap_or(token).split('.');

        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;

        Some(Self::new(major, minor, patch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_with_git_style_prefix() {
        let version = Version::parse("git version 2.51.0").unwrap();

        assert_eq!(version, Version::new(2, 51, 0));
    }

    #[test]
    fn parses_version_with_trailing_text() {
        let version = Version::parse("gh version 2.40.1 (2023-12-13)").unwrap();

        assert_eq!(version, Version::new(2, 40, 1));
    }

    #[test]
    fn parses_v_prefixed_tags() {
        let version = Version::parse("gh stack\tgithub/gh-stack\tv0.5.0").unwrap();

        assert_eq!(version, Version::new(0, 5, 0));
    }

    #[test]
    fn rejects_input_with_no_version_number() {
        let result = Version::parse("hello world");

        assert!(result.is_err());
    }

    #[test]
    fn compares_by_major_then_minor_then_patch() {
        assert!(Version::new(2, 51, 0) > Version::new(2, 20, 0));
        assert!(Version::new(2, 20, 1) > Version::new(2, 20, 0));
        assert!(Version::new(1, 99, 99) < Version::new(2, 0, 0));
        assert_eq!(Version::new(2, 20, 0), Version::new(2, 20, 0));
    }
}
