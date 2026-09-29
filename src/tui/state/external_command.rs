//! A request to hand the real terminal to another program.
//!
//! Some commands can't run as a background effect because they need the
//! user's terminal: `$EDITOR` on a conflicted file, or an interactive
//! command such as `gh stack modify`. The reducer can't run them (it never
//! awaits, and the TUI owns the terminal), so it stores one of these in
//! [`super::AppState::external_command`]. The event loop in
//! `tui::app::run_app` then takes it, suspends the TUI, runs the program
//! with inherited stdin/stdout/stderr, restores the TUI, and dispatches
//! [`Action::ExternalCommandFinished`] carrying [`ExternalCommand::then`].

use crate::tui::action::Action;

#[derive(Debug, Clone)]
pub struct ExternalCommand {
    /// Prefixes the error if the command can't start or exits non-zero,
    /// e.g. `"open editor"`.
    pub context: String,
    pub program: String,
    pub args: Vec<String>,
    /// Dispatched once the command has exited, whether or not it succeeded
    /// (e.g. reload whatever the command may have changed).
    pub then: Box<Action>,
}

impl ExternalCommand {
    pub fn new(
        context: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
        then: Action,
    ) -> Self {
        Self {
            context: context.into(),
            program: program.into(),
            args,
            then: Box::new(then),
        }
    }

    /// Opens `path` in the user's editor: `$EDITOR` (which may carry its own
    /// arguments, e.g. `code --wait`), else `vi`.
    pub fn editor(editor_env: Option<&str>, path: &str, then: Action) -> Self {
        let mut words = editor_env
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string);
        let program = words.next().unwrap_or_else(|| "vi".to_string());
        let mut args: Vec<String> = words.collect();
        args.push(path.to_string());
        Self::new("open editor", program, args, then)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_uses_editor_env_with_its_arguments() {
        let command = ExternalCommand::editor(Some("code --wait"), "src/a.rs", Action::Quit);

        assert_eq!(command.program, "code");
        assert_eq!(command.args, ["--wait", "src/a.rs"]);
        assert!(matches!(*command.then, Action::Quit));
    }

    #[test]
    fn editor_falls_back_to_vi() {
        for unset in [None, Some(""), Some("   ")] {
            let command = ExternalCommand::editor(unset, "a b.txt", Action::Quit);
            assert_eq!(command.program, "vi");
            assert_eq!(command.args, ["a b.txt"]);
        }
    }
}
