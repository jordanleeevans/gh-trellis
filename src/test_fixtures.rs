use crate::stack::{Layer, StackSummary};
use crate::tui::state::{AppState, Screen};

pub fn layer(name: &str) -> Layer {
    Layer {
        branch: name.to_string(),
        head: None,
        base: "main".to_string(),
        is_current: false,
        is_merged: false,
        is_queued: false,
        needs_rebase: false,
        pull_request: None,
        commits: Vec::new(),
        position: 0,
    }
}

pub fn stack_summary(label: &str, layer_count: usize) -> StackSummary {
    StackSummary {
        label: label.to_string(),
        trunk: "main".to_string(),
        layers: (0..layer_count)
            .map(|index| layer(&format!("{label}-layer-{index}")))
            .collect(),
        is_current: false,
    }
}

/// Runs `git` in `repo` for fixture setup, returning whether it succeeded.
fn fixture_git(repo: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git must be installed to run the rebase fixture tests")
        .success()
}

/// The text `feature` gives `shared.txt` in [`conflicting_rebase_repo`].
pub const FIXTURE_FEATURE_TEXT: &str = "feature\n";

/// A throwaway repository, with no remote, stopped on a real rebase
/// conflict.
///
/// `main` and `feature` both change `shared.txt` from a common base, and
/// `feature` has a second, non-conflicting commit (`feature-only.txt`).
/// `feature` is checked out and `git rebase main` has stopped on
/// `shared.txt`. Commit signing, hooks and rerere are turned off in the
/// repo's own config so the user's global git config can't interfere.
///
/// This uses plain `git rebase`: `gh stack rebase` needs gh-stack's local
/// stack file and fetches from a remote, so it can't run offline.
pub fn conflicting_rebase_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let hooks = repo.join(".no-hooks");
    std::fs::create_dir(&hooks).unwrap();
    let write = |name: &str, text: &str| std::fs::write(repo.join(name), text).unwrap();
    let git = |args: &[&str]| assert!(fixture_git(repo, args), "git {args:?} failed");

    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "rerere.enabled", "false"]);
    git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    write(".gitignore", ".no-hooks/\n");
    write("shared.txt", "base\n");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);

    git(&["checkout", "-q", "-b", "feature"]);
    write("shared.txt", FIXTURE_FEATURE_TEXT);
    git(&["commit", "-q", "-am", "feature: change shared"]);
    write("feature-only.txt", "only on feature\n");
    git(&["add", "feature-only.txt"]);
    git(&["commit", "-q", "-m", "feature: add file"]);

    git(&["checkout", "-q", "main"]);
    write("shared.txt", "main\n");
    git(&["commit", "-q", "-am", "main: change shared"]);

    git(&["checkout", "-q", "feature"]);
    assert!(
        !fixture_git(repo, &["rebase", "main"]),
        "the fixture rebase should stop on a conflict"
    );
    dir
}

pub fn app_state(stacks: Vec<StackSummary>, screen: Screen) -> AppState {
    AppState {
        stacks,
        screen,
        ..AppState::default()
    }
}
