//! The app shell: owns [`AppState`] and the components, runs the event
//! loop, and applies [`Action`]s in a synchronous reducer. Anything that
//! runs a process is handed to [`Effects`], whose results come back over a
//! channel as further `Action`s, so the render loop never blocks.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::shell::Shell;

mod auto_refresh;
mod feedback;
mod layers;
mod refresh;
mod stack_ops;
mod submit;
mod sync;
#[cfg(test)]
pub(crate) mod test_support;

use super::action::Action;
use super::component::Component;
use super::components::confirm::{self, ConfirmModal};
use super::components::stack_browser::StackBrowser;
use super::effects::Effects;
use super::keymap::{KeyIntent, key_intent};
use super::state::{AppState, Screen};
use auto_refresh::AutoRefresh;

struct App {
    state: AppState,
    stack_browser: StackBrowser,
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::default(),
            stack_browser: StackBrowser::new(),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.stack_browser.draw(frame, &self.state);

        if let Some(modal) = &self.state.confirm {
            confirm::render(frame, frame.area(), modal);
        }
    }

    /// Turns a raw key event into `Action`s. A confirm modal, when shown,
    /// gets first refusal on every key: input is routed to it instead of
    /// falling through to the active screen's own key handling, exactly the
    /// way `stack_browser` already intercepts keys internally based on its
    /// own state (e.g. its pending-`g` handling).
    fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if let Some(modal) = &self.state.confirm {
            return confirm::handle_confirm_key(modal, key);
        }

        // A status message lasts until the next key press. Clear it first,
        // so a status set by this key's own actions survives.
        let mut actions = Vec::new();
        if self.state.status.is_some() {
            actions.push(Action::ClearStatus);
        }

        actions.extend(
            self.stack_browser
                .handle_key(key, &self.state)
                .into_iter()
                .map(wrap_quit_in_confirmation),
        );

        if self.state.error.is_some() && key_intent(key) == Some(KeyIntent::DismissMessage) {
            actions.push(Action::ClearError);
        }

        actions
    }

    /// Applies `actions` and every follow-up they produce, in order.
    fn dispatch(&mut self, actions: Vec<Action>, effects: &Effects) {
        let mut pending = std::collections::VecDeque::from(actions);

        while let Some(action) = pending.pop_front() {
            let follow_ups = self.apply_action(&action, effects);
            self.stack_browser.update(&action, &mut self.state);
            pending.extend(follow_ups);

            // Whenever the selected layer may have changed (including when a
            // fresh stack list first selects one), load its detail and diff.
            // Both are cached, so an unchanged selection costs nothing.
            if matches!(
                action,
                Action::SelectNext | Action::SelectPrevious | Action::StacksLoaded(_)
            ) && let Screen::Layers(stack_index) = self.state.screen
                && let Some(layer_index) = self.stack_browser.selected_index()
            {
                pending.push_back(Action::LoadLayerDetail {
                    stack_index,
                    layer_index,
                    force: false,
                });
                pending.push_back(Action::LoadLayerDiff {
                    stack_index,
                    layer_index,
                    force: false,
                });
            }
        }
    }

    /// The reducer: updates state for `action`, hands any process work to
    /// `effects`, and returns follow-up actions to apply immediately.
    fn apply_action(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::Quit => {
                self.state.should_quit = true;
                Vec::new()
            }
            Action::RefreshStacks
            | Action::StackRefreshStarted { .. }
            | Action::StackRefreshSucceeded { .. }
            | Action::Tick
            | Action::StacksLoaded(_) => self.reduce_refresh(action, effects),
            Action::ShowLayers(_)
            | Action::LoadLayerDetail { .. }
            | Action::LayerDetailLoaded { .. }
            | Action::LoadLayerDiff { .. }
            | Action::LayerDiffLoaded { .. } => self.reduce_layers(action, effects),
            Action::CheckoutSelected { .. }
            | Action::AddLayer { .. }
            | Action::UnstackSelected { .. }
            | Action::RunUnstack { .. }
            | Action::OpenPullRequest { .. } => self.reduce_stack_ops(action, effects),
            Action::ClearStatus
            | Action::SetError(_)
            | Action::ClearError
            | Action::ShowConfirm(_)
            | Action::ConfirmCancel
            | Action::ConfirmAccept
            | Action::ConfirmInput(_)
            | Action::ConfirmBackspace => self.reduce_feedback(action),
            Action::SubmitStack { .. }
            | Action::SubmitStarted { .. }
            | Action::SubmitLayerProgress { .. }
            | Action::SubmitFinished { .. }
            | Action::ToggleSubmitAuto
            | Action::ToggleSubmitOpen
            | Action::DismissSubmit => self.reduce_submit(action, effects),
            Action::SyncStack { .. }
            | Action::SyncStarted { .. }
            | Action::SyncFinished { .. }
            | Action::ToggleSyncPrune => self.reduce_sync(action, effects),
            // View-only actions: handled by the components' `update`.
            Action::SelectNext
            | Action::SelectPrevious
            | Action::FocusNextPanel
            | Action::FocusPreviousPanel
            | Action::SelectNextDiffFile
            | Action::SelectPreviousDiffFile
            | Action::ScrollDiffLineDown
            | Action::ScrollDiffLineUp
            | Action::ScrollDiffDown
            | Action::ScrollDiffUp
            | Action::ScrollDiffHalfPageDown
            | Action::ScrollDiffHalfPageUp
            | Action::ScrollDiffTop
            | Action::ScrollDiffBottom
            | Action::ToggleDiffView => Vec::new(),
        }
    }
}

/// Wraps a raw `Action::Quit` (as produced by the active screen's own key
/// handling, e.g. `q`/Esc in `stack_browser`) in a non-danger confirm modal
/// instead of letting it fire immediately, so accidentally hitting `q`
/// doesn't kill the whole TUI. `Action::Quit` itself is untouched and still
/// performs the actual quit once the modal is confirmed, since this rewrite
/// only happens at the key-handling boundary, not inside the reducer, so
/// replaying `Action::Quit` from the modal's `on_confirm` cannot loop back
/// into another confirmation.
fn wrap_quit_in_confirmation(action: Action) -> Action {
    match action {
        Action::Quit => Action::ShowConfirm(
            ConfirmModal::new(
                "Quit trellis?",
                "Any in-flight background refresh will be cancelled.",
                "Quit",
                false,
            )
            .on_confirm(Action::Quit),
        ),
        other => other,
    }
}

/// Runs the TUI until the user quits, then restores the terminal.
pub async fn run(shell: Arc<dyn Shell>, repo: &Path) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, shell, repo).await;
    ratatui::restore();
    result
}

async fn run_app(
    terminal: &mut DefaultTerminal,
    shell: Arc<dyn Shell>,
    repo: &Path,
) -> anyhow::Result<()> {
    let mut app = App::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let effects = Effects::new(repo.to_path_buf(), shell, tx);
    let mut auto_refresh =
        AutoRefresh::new(crate::config::get().refresh_interval_secs, Instant::now());
    app.dispatch(vec![Action::RefreshStacks], &effects);

    while !app.state.should_quit {
        while let Ok(action) = rx.try_recv() {
            app.dispatch(vec![action], &effects);
        }

        app.dispatch(vec![Action::Tick], &effects);
        if auto_refresh.due(Instant::now(), &app.state) {
            app.dispatch(vec![Action::RefreshStacks], &effects);
        }

        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let actions = app.handle_key(key);
            app.dispatch(actions, &effects);
        }
    }

    Ok(())
}
