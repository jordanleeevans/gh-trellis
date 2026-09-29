//! What to run for each kind of failure.

use super::error::{CheckFailure, Tool};

/// The operating system family, used to pick a package manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        Self::from_os(std::env::consts::OS)
    }

    /// Maps a `std::env::consts::OS` value to a platform.
    pub fn from_os(os: &str) -> Self {
        match os {
            "macos" => Platform::MacOs,
            "linux" => Platform::Linux,
            "windows" => Platform::Windows,
            _ => Platform::Other,
        }
    }
}

/// Exact commands to fix a failure, plus a docs link to fall back on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remedy {
    pub commands: Vec<String>,
    pub docs: &'static str,
}

/// The install docs link for a tool.
pub fn docs_url(tool: Tool) -> &'static str {
    match tool {
        Tool::Git => "https://git-scm.com/downloads",
        Tool::Gh => "https://cli.github.com",
        Tool::GhStack => "https://github.com/github/gh-stack#installation",
        Tool::GhAuth => "https://cli.github.com/manual/gh_auth_login",
    }
}

/// The commands to run to resolve `failure` on `platform`.
pub fn remedy(failure: &CheckFailure, platform: Platform) -> Remedy {
    let tool = failure.tool();
    // A binary that's present but old or unreadable needs an upgrade; one
    // that's missing needs an install. The extension and auth checks don't
    // fit this split, so they handle it themselves.
    let upgrade = matches!(
        failure,
        CheckFailure::OutdatedVersion { .. } | CheckFailure::UnparseableVersion { .. }
    );
    let commands = match tool {
        Tool::GhAuth => vec!["gh auth login".to_string()],
        Tool::GhStack if upgrade => vec!["gh extension upgrade gh-stack".to_string()],
        Tool::GhStack => vec!["gh extension install github/gh-stack".to_string()],
        Tool::Git => package_commands(platform, upgrade, "git", "Git.Git", "git"),
        Tool::Gh => package_commands(platform, upgrade, "gh", "GitHub.cli", "gh"),
    };
    Remedy {
        commands,
        docs: docs_url(tool),
    }
}

fn package_commands(
    platform: Platform,
    upgrade: bool,
    brew: &str,
    winget_id: &str,
    apt_package: &str,
) -> Vec<String> {
    let verb = if upgrade { "upgrade" } else { "install" };
    match platform {
        Platform::MacOs => vec![format!("brew {verb} {brew}")],
        Platform::Windows => vec![format!("winget {verb} --id {winget_id} -e")],
        // gh isn't in the default apt/dnf repositories, so it has no
        // one-liner that's right everywhere; the docs link covers it.
        Platform::Linux if apt_package == "gh" => Vec::new(),
        Platform::Linux => {
            let apt = if upgrade {
                format!("sudo apt install --only-upgrade {apt_package}")
            } else {
                format!("sudo apt install {apt_package}")
            };
            let dnf = format!("sudo dnf {verb} {apt_package}");
            let pacman = format!("sudo pacman -S {apt_package}");
            vec![
                format!("{apt}    # Debian, Ubuntu"),
                format!("{dnf}    # Fedora"),
                format!("{pacman}    # Arch"),
            ]
        }
        Platform::Other => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::version::Version;

    fn missing(tool: Tool) -> CheckFailure {
        CheckFailure::NotInstalled { tool }
    }

    fn old(tool: Tool) -> CheckFailure {
        CheckFailure::OutdatedVersion {
            tool,
            found: Version::new(0, 0, 1),
            minimum: Version::new(1, 0, 0),
        }
    }

    #[test]
    fn missing_gh_stack_gets_the_install_command_on_every_platform() {
        for platform in [
            Platform::MacOs,
            Platform::Linux,
            Platform::Windows,
            Platform::Other,
        ] {
            assert_eq!(
                remedy(&missing(Tool::GhStack), platform).commands,
                ["gh extension install github/gh-stack"]
            );
        }
    }

    #[test]
    fn outdated_gh_stack_gets_the_upgrade_command() {
        assert_eq!(
            remedy(&old(Tool::GhStack), Platform::MacOs).commands,
            ["gh extension upgrade gh-stack"]
        );
    }

    #[test]
    fn unauthenticated_gets_gh_auth_login() {
        let failure = CheckFailure::Failed {
            tool: Tool::GhAuth,
            reason: "exit 1".to_string(),
        };

        assert_eq!(
            remedy(&failure, Platform::Linux).commands,
            ["gh auth login"]
        );
    }

    #[test]
    fn missing_git_is_platform_aware() {
        assert_eq!(
            remedy(&missing(Tool::Git), Platform::MacOs).commands,
            ["brew install git"]
        );
        assert_eq!(
            remedy(&missing(Tool::Git), Platform::Windows).commands,
            ["winget install --id Git.Git -e"]
        );
        let linux = remedy(&missing(Tool::Git), Platform::Linux).commands;
        assert!(linux[0].starts_with("sudo apt install git"));
        assert!(linux[1].starts_with("sudo dnf install git"));
        assert!(linux[2].starts_with("sudo pacman -S git"));
    }

    #[test]
    fn missing_gh_is_platform_aware() {
        assert_eq!(
            remedy(&missing(Tool::Gh), Platform::MacOs).commands,
            ["brew install gh"]
        );
        assert_eq!(
            remedy(&missing(Tool::Gh), Platform::Windows).commands,
            ["winget install --id GitHub.cli -e"]
        );
    }

    #[test]
    fn outdated_binaries_get_upgrade_commands() {
        assert_eq!(
            remedy(&old(Tool::Gh), Platform::MacOs).commands,
            ["brew upgrade gh"]
        );
        assert_eq!(
            remedy(&old(Tool::Git), Platform::Windows).commands,
            ["winget upgrade --id Git.Git -e"]
        );
        assert!(
            remedy(&old(Tool::Git), Platform::Linux).commands[0]
                .starts_with("sudo apt install --only-upgrade git")
        );
    }

    #[test]
    fn platforms_without_a_one_liner_fall_back_to_docs() {
        let r = remedy(&missing(Tool::Gh), Platform::Linux);

        assert!(r.commands.is_empty());
        assert_eq!(r.docs, "https://cli.github.com");
        assert!(
            remedy(&missing(Tool::Git), Platform::Other)
                .commands
                .is_empty()
        );
    }

    #[test]
    fn platform_maps_os_names() {
        assert_eq!(Platform::from_os("macos"), Platform::MacOs);
        assert_eq!(Platform::from_os("linux"), Platform::Linux);
        assert_eq!(Platform::from_os("windows"), Platform::Windows);
        assert_eq!(Platform::from_os("freebsd"), Platform::Other);
    }
}
