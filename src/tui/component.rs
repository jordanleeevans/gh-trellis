use crossterm::event::KeyEvent;
use ratatui::Frame;

use super::action::Action;
use super::state::AppState;

/// A panel or overlay: turns keys into [`Action`]s, reacts to applied
/// actions, and draws itself from [`AppState`]. Components never touch a
/// [`crate::shell::Shell`]; anything that runs a process is an `Action`
/// handled by [`super::effects`].
pub trait Component {
    fn draw(&mut self, frame: &mut Frame, state: &AppState);
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action>;
    fn update(&mut self, action: &Action, state: &mut AppState);
}
