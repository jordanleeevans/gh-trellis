//! Shared harness for reducer tests: drives actions through the real
//! reducer and effect runner against a [`MockShell`].

pub(crate) use std::sync::Arc;

pub(crate) use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub(crate) use tokio::sync::mpsc;

pub(crate) use crate::shell::{MockShell, ShellError};
pub(crate) use crate::stack::StackSummary;
pub(crate) use crate::test_fixtures::stack_summary;
pub(crate) use crate::tui::action::Action;
pub(crate) use crate::tui::components::confirm::ConfirmModal;
pub(crate) use crate::tui::effects::Effects;
pub(crate) use crate::tui::state::submit_progress::LayerSubmitStatus;

use super::App;

pub(crate) fn test_effects(shell: &MockShell) -> (Effects, mpsc::UnboundedReceiver<Action>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let repo = std::env::current_dir().unwrap();
    (Effects::new(repo, Arc::new(shell.clone()), tx), rx)
}

impl App {
    /// Applies `action` without letting any effect it spawns run: the
    /// single-threaded test runtime never polls the spawned task.
    pub(super) fn apply(&mut self, action: &Action) -> Vec<Action> {
        let (effects, _rx) = test_effects(&MockShell::new());
        self.apply_action(action, &effects)
    }

    /// Applies `action`, runs its effects against `shell` to completion,
    /// and returns its immediate follow-ups followed by every action the
    /// effects sent back (neither is applied).
    pub(super) async fn settle(&mut self, action: &Action, shell: &MockShell) -> Vec<Action> {
        let (effects, mut rx) = test_effects(shell);
        let mut actions = self.apply_action(action, &effects);
        effects.wait_idle().await;
        while let Ok(action) = rx.try_recv() {
            actions.push(action);
        }
        actions
    }

    /// Dispatches `actions` and their follow-ups, without running effects.
    pub(super) fn dispatch_now(&mut self, actions: Vec<Action>) {
        let (effects, _rx) = test_effects(&MockShell::new());
        self.dispatch(actions, &effects);
    }
}

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
