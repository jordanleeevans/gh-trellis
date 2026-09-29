//! Handing the terminal to another program (`$EDITOR`, and later
//! interactive commands such as `gh stack modify`).
//!
//! The reducer only records the request ([`Action::RunExternal`] sets
//! `AppState::external_command`). The event loop in `run_app` picks it up
//! between frames and calls [`suspend_and_run`]:
//!
//! 1. `ratatui::restore()` leaves raw mode and the alternate screen, and
//!    the cursor (hidden while drawing) is shown again, so the program gets
//!    a normal terminal.
//! 2. Run the program with inherited stdin, stdout and stderr, and wait for
//!    it. The loop isn't polling crossterm events meanwhile, so every key
//!    goes to the program. Background effects keep running and their
//!    results queue in the channel until the loop resumes.
//! 3. Re-enter raw mode and the alternate screen, and clear the terminal so
//!    the next draw repaints every cell instead of diffing against a buffer
//!    the program has overwritten. This re-enters by hand instead of calling
//!    `ratatui::init()` again, which would install a second panic hook and
//!    build a new `Terminal`; the existing one is still valid.
//! 4. Dispatch [`Action::ExternalCommandFinished`] with the command's
//!    `then` action.
//!
//! Step 3 runs whether or not the program could be started, so a missing
//! editor can't leave the terminal outside raw mode.

use std::io;
use std::path::Path;
use std::process::Stdio;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::DefaultTerminal;

use crate::tui::action::Action;
use crate::tui::state::external_command::ExternalCommand;

use super::App;

impl App {
    pub(super) fn reduce_external(&mut self, action: &Action) -> Vec<Action> {
        match action {
            Action::RunExternal(command) => {
                if self.state.external_command.is_some() {
                    return vec![Action::SetError(format!(
                        "{}: another command is already waiting for the terminal",
                        command.context
                    ))];
                }
                self.state.external_command = Some(command.clone());
                Vec::new()
            }
            Action::ExternalCommandFinished { result, then } => match result {
                Ok(()) => vec![(**then).clone()],
                Err(message) => vec![Action::SetError(message.clone()), (**then).clone()],
            },
            _ => unreachable!("routed to reduce_external by App::apply_action"),
        }
    }
}

/// Suspends the TUI, runs `command` in `cwd` on the real terminal, and
/// resumes. The outer `Err` means the terminal couldn't be restored (fatal);
/// the inner result is the command's own, already user-facing.
pub(super) async fn suspend_and_run(
    terminal: &mut DefaultTerminal,
    command: &ExternalCommand,
    cwd: &Path,
) -> io::Result<Result<(), String>> {
    ratatui::restore();
    let _ = terminal.show_cursor();

    let result = run_inherited(command, cwd).await;

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(result)
}

/// Runs `command` in `cwd` with the parent's stdin, stdout and stderr, and
/// waits for it to exit.
async fn run_inherited(command: &ExternalCommand, cwd: &Path) -> Result<(), String> {
    let status = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await;
    let context = &command.context;
    let program = &command.program;
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(match status.code() {
            Some(code) => format!("{context}: `{program}` exited with code {code}"),
            None => format!("{context}: `{program}` was terminated by a signal"),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(format!(
            "{context}: `{program}` not found (set $EDITOR to your editor)"
        )),
        Err(error) => Err(format!("{context}: could not start `{program}`: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str]) -> ExternalCommand {
        ExternalCommand::new(
            "open editor",
            program,
            args.iter().map(|arg| arg.to_string()).collect(),
            Action::LoadRebaseState,
        )
    }

    #[test]
    fn run_external_is_queued_for_the_event_loop() {
        let mut app = App::new();

        let follow_ups = app.apply(&Action::RunExternal(command("vi", &["a.rs"])));

        assert!(follow_ups.is_empty());
        assert_eq!(app.state.external_command.as_ref().unwrap().program, "vi");

        let second = app.apply(&Action::RunExternal(command("vi", &["b.rs"])));
        assert!(matches!(second.as_slice(), [Action::SetError(_)]));
        assert_eq!(app.state.external_command.as_ref().unwrap().args, ["a.rs"]);
    }

    #[test]
    fn finishing_dispatches_the_follow_up_even_after_a_failure() {
        let mut app = App::new();
        let then = Box::new(Action::LoadRebaseState);

        let ok = app.apply(&Action::ExternalCommandFinished {
            result: Ok(()),
            then: then.clone(),
        });
        let failed = app.apply(&Action::ExternalCommandFinished {
            result: Err("open editor: `vi` exited with code 1".to_string()),
            then,
        });

        assert!(matches!(ok.as_slice(), [Action::LoadRebaseState]));
        assert!(matches!(
            failed.as_slice(),
            [Action::SetError(_), Action::LoadRebaseState]
        ));
    }

    #[tokio::test]
    async fn run_inherited_runs_in_the_given_directory_and_reports_exit_codes() {
        let dir = tempfile::tempdir().unwrap();

        let ok = run_inherited(&command("sh", &["-c", "touch ran"]), dir.path()).await;
        let failed = run_inherited(&command("sh", &["-c", "exit 3"]), dir.path()).await;
        let missing = run_inherited(&command("trellis-no-such-editor", &[]), dir.path()).await;

        assert_eq!(ok, Ok(()));
        assert!(dir.path().join("ran").exists());
        assert_eq!(
            failed,
            Err("open editor: `sh` exited with code 3".to_string())
        );
        assert!(missing.unwrap_err().contains("not found"));
    }
}
