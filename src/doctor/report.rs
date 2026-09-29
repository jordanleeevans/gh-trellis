//! Renders a [`Diagnosis`] as the first-run report shown before the TUI
//! starts. Pure: takes data in, returns a `String`, so it's tested directly.

use std::ffi::OsStr;

use super::error::{CheckFailure, Tool};
use super::remedy::{Platform, remedy};
use super::{Diagnosis, Finding, Status};

const BOLD: &str = "1";
const DIM: &str = "2";
const RED: &str = "31";
const GREEN: &str = "32";
const YELLOW: &str = "33";
const CYAN: &str = "36";

/// Whether to emit ANSI colors. Off when the output isn't a terminal, when
/// `NO_COLOR` is set to a non-empty value (https://no-color.org), or when
/// `TERM=dumb`.
pub fn use_color(is_terminal: bool, no_color: Option<&OsStr>, term: Option<&OsStr>) -> bool {
    is_terminal
        && no_color.is_none_or(|value| value.is_empty())
        && term.is_none_or(|value| value != "dumb")
}

/// Renders the report, or `None` when everything passed (nothing to say).
pub fn render(diagnosis: &Diagnosis, platform: Platform, color: bool) -> Option<String> {
    if diagnosis.is_healthy() {
        return None;
    }
    let paint = Painter { color };
    let mut out = String::new();

    let failing = diagnosis
        .0
        .iter()
        .filter(|f| matches!(f.status, Status::Failed(_)))
        .count();
    let noun = if failing == 1 {
        "requirement needs"
    } else {
        "requirements need"
    };
    out.push_str(&paint.paint(
        BOLD,
        &format!("trellis can't start yet: {failing} {noun} attention."),
    ));
    out.push_str("\n\n");

    for finding in &diagnosis.0 {
        out.push_str(&checklist_row(finding, &paint));
        out.push('\n');
    }

    let fixes: Vec<&CheckFailure> = diagnosis
        .0
        .iter()
        .filter_map(|f| match &f.status {
            Status::Failed(failure) => Some(failure),
            _ => None,
        })
        .collect();

    out.push('\n');
    out.push_str(&paint.paint(BOLD, "How to fix"));
    out.push('\n');
    for failure in fixes {
        let r = remedy(failure, platform);
        out.push('\n');
        out.push_str(&format!(
            "  {} {}\n",
            paint.paint(BOLD, failure.tool().name()),
            paint.paint(DIM, &format!("({})", problem(failure))),
        ));
        for command in &r.commands {
            out.push_str(&format!("    $ {}\n", paint.paint(CYAN, command)));
        }
        out.push_str(&format!("    docs: {}\n", paint.paint(CYAN, r.docs)));
    }

    out.push('\n');
    out.push_str("Fix the above, then run trellis again.\n");
    Some(out)
}

fn checklist_row(finding: &Finding, paint: &Painter) -> String {
    let (mark, code) = match finding.status {
        Status::Ok(_) => ("✓", GREEN),
        Status::Failed(_) => ("✗", RED),
        Status::Blocked => ("-", YELLOW),
    };
    let need = finding
        .minimum
        .map(|min| format!("need {min}+"))
        .unwrap_or_default();
    let detail = match &finding.status {
        Status::Ok(Some(found)) => format!("found {found}, {need}"),
        Status::Ok(None) if finding.tool == Tool::GhAuth => "logged in".to_string(),
        Status::Ok(None) => "installed".to_string(),
        Status::Failed(CheckFailure::OutdatedVersion { found, minimum, .. }) => {
            format!("found {found}, need {minimum}+ (too old)")
        }
        Status::Failed(failure) => problem_with_need(failure, &need),
        Status::Blocked => "not checked until gh is installed".to_string(),
    };
    format!(
        "  {} {} {}",
        paint.paint(code, mark),
        paint.paint(BOLD, &format!("{:<9}", finding.tool.name())),
        detail,
    )
}

fn problem_with_need(failure: &CheckFailure, need: &str) -> String {
    let problem = problem(failure);
    if need.is_empty() {
        problem
    } else {
        format!("{problem} ({need})")
    }
}

/// A short description of what's wrong.
fn problem(failure: &CheckFailure) -> String {
    match failure {
        CheckFailure::NotInstalled { .. } => "not installed".to_string(),
        CheckFailure::OutdatedVersion { found, minimum, .. } => {
            format!("found {found}, need {minimum}+")
        }
        CheckFailure::UnparseableVersion { .. } => "couldn't read its version".to_string(),
        CheckFailure::Failed {
            tool: Tool::GhAuth, ..
        } => "not logged in".to_string(),
        CheckFailure::Failed { reason, .. } => {
            format!(
                "check failed: {}",
                reason.lines().next().unwrap_or_default()
            )
        }
    }
}

struct Painter {
    color: bool,
}

impl Painter {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::version::Version;

    fn found(tool: Tool, minimum: Option<Version>, version: Option<Version>) -> Finding {
        Finding {
            tool,
            minimum,
            status: Status::Ok(version),
        }
    }

    fn failed(tool: Tool, minimum: Option<Version>, failure: CheckFailure) -> Finding {
        Finding {
            tool,
            minimum,
            status: Status::Failed(failure),
        }
    }

    fn healthy() -> Diagnosis {
        Diagnosis(vec![
            found(
                Tool::Git,
                Some(Version::new(2, 20, 0)),
                Some(Version::new(2, 43, 0)),
            ),
            found(
                Tool::Gh,
                Some(Version::new(2, 90, 0)),
                Some(Version::new(2, 91, 0)),
            ),
            found(
                Tool::GhStack,
                Some(Version::new(0, 1, 0)),
                Some(Version::new(0, 5, 0)),
            ),
            found(Tool::GhAuth, None, None),
        ])
    }

    fn missing_stack() -> Diagnosis {
        let mut d = healthy();
        d.0[2] = failed(
            Tool::GhStack,
            Some(Version::new(0, 1, 0)),
            CheckFailure::NotInstalled {
                tool: Tool::GhStack,
            },
        );
        d
    }

    fn plain(d: &Diagnosis) -> String {
        render(d, Platform::MacOs, false).unwrap()
    }

    #[test]
    fn healthy_machine_has_no_report() {
        assert_eq!(render(&healthy(), Platform::MacOs, true), None);
    }

    #[test]
    fn missing_gh_stack_prints_the_exact_install_command() {
        let report = plain(&missing_stack());

        assert!(report.contains("$ gh extension install github/gh-stack\n"));
        assert!(report.contains("https://github.com/github/gh-stack#installation"));
        assert!(report.contains("1 requirement needs attention"));
    }

    #[test]
    fn checklist_shows_every_requirement_with_versions() {
        let report = plain(&missing_stack());

        assert!(report.contains("✓ git       found 2.43.0, need 2.20.0+"));
        assert!(report.contains("✓ gh        found 2.91.0, need 2.90.0+"));
        assert!(report.contains("✗ gh stack  not installed (need 0.1.0+)"));
        assert!(report.contains("✓ gh auth   logged in"));
    }

    #[test]
    fn outdated_version_shows_found_versus_required() {
        let mut d = healthy();
        d.0[1] = failed(
            Tool::Gh,
            Some(Version::new(2, 90, 0)),
            CheckFailure::OutdatedVersion {
                tool: Tool::Gh,
                found: Version::new(2, 40, 1),
                minimum: Version::new(2, 90, 0),
            },
        );

        let report = plain(&d);

        assert!(report.contains("found 2.40.1, need 2.90.0+ (too old)"));
        assert!(report.contains("$ brew upgrade gh\n"));
    }

    #[test]
    fn unauthenticated_prints_gh_auth_login() {
        let mut d = healthy();
        d.0[3] = failed(
            Tool::GhAuth,
            None,
            CheckFailure::Failed {
                tool: Tool::GhAuth,
                reason: "exit 1".to_string(),
            },
        );

        let report = plain(&d);

        assert!(report.contains("✗ gh auth   not logged in"));
        assert!(report.contains("$ gh auth login\n"));
    }

    #[test]
    fn multiple_failures_are_all_listed_with_their_commands() {
        let d = Diagnosis(vec![
            failed(
                Tool::Git,
                Some(Version::new(2, 20, 0)),
                CheckFailure::NotInstalled { tool: Tool::Git },
            ),
            failed(
                Tool::Gh,
                Some(Version::new(2, 90, 0)),
                CheckFailure::NotInstalled { tool: Tool::Gh },
            ),
            Finding {
                tool: Tool::GhStack,
                minimum: Some(Version::new(0, 1, 0)),
                status: Status::Blocked,
            },
            Finding {
                tool: Tool::GhAuth,
                minimum: None,
                status: Status::Blocked,
            },
        ]);

        let report = plain(&d);

        assert!(report.contains("2 requirements need attention"));
        assert!(report.contains("$ brew install git\n"));
        assert!(report.contains("$ brew install gh\n"));
        assert!(report.contains("- gh stack  not checked until gh is installed"));
        assert!(report.contains("https://git-scm.com/downloads"));
        assert!(report.contains("https://cli.github.com"));
    }

    #[test]
    fn commands_follow_the_platform() {
        let d = Diagnosis(vec![failed(
            Tool::Git,
            Some(Version::new(2, 20, 0)),
            CheckFailure::NotInstalled { tool: Tool::Git },
        )]);

        let windows = render(&d, Platform::Windows, false).unwrap();

        assert!(windows.contains("$ winget install --id Git.Git -e\n"));
    }

    #[test]
    fn plain_report_has_no_escape_codes_and_colored_report_does() {
        let d = missing_stack();

        assert!(!render(&d, Platform::MacOs, false).unwrap().contains('\x1b'));
        assert!(
            render(&d, Platform::MacOs, true)
                .unwrap()
                .contains("\x1b[31m")
        );
    }

    #[test]
    fn color_needs_a_terminal() {
        assert!(use_color(true, None, None));
        assert!(!use_color(false, None, None));
    }

    #[test]
    fn no_color_env_disables_color() {
        assert!(!use_color(true, Some(OsStr::new("1")), None));
        // An empty NO_COLOR doesn't count, per the spec.
        assert!(use_color(true, Some(OsStr::new("")), None));
    }

    #[test]
    fn dumb_terminal_disables_color() {
        assert!(!use_color(true, None, Some(OsStr::new("dumb"))));
        assert!(use_color(true, None, Some(OsStr::new("xterm-256color"))));
    }
}
