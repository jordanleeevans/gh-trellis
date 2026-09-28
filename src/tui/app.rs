//! The app shell: owns [`AppState`] and the components, runs the event
//! loop, and applies [`Action`]s in a synchronous reducer. Anything that
//! runs a process is handed to [`Effects`], whose results come back over a
//! channel as further `Action`s, so the render loop never blocks.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::shell::Shell;
use crate::stack::{Layer, StackSummary};

use super::action::Action;
use super::component::Component;
use super::components::confirm::{self, ConfirmModal};
use super::components::stack_browser::StackBrowser;
use super::effects::Effects;
use super::keymap::{KeyIntent, key_intent};
use super::messages::friendly_stack_refresh_error;
use super::state::submit_progress::SubmitProgress;
use super::state::{
    AppState, Screen, layer_detail_cache_key, layer_diff_cache_key, lower_layer_ref,
};

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

    fn selected_stack_label(&self) -> Option<String> {
        match self.state.screen {
            Screen::Layers(index) => self
                .state
                .stacks
                .get(index)
                .map(|stack| stack.label.clone()),
            Screen::List => None,
        }
    }

    fn apply_stacks_loaded(&mut self, stacks: Vec<StackSummary>) -> Vec<Action> {
        let selected_label = self.selected_stack_label();
        let selected_index = selected_label
            .and_then(|label| stacks.iter().position(|stack| stack.label == label))
            .or_else(|| stacks.iter().position(|stack| stack.is_current))
            .or(if stacks.is_empty() { None } else { Some(0) });

        self.state.stacks = stacks;
        self.state.last_successful_stacks = self.state.stacks.clone();
        self.state.screen = match selected_index {
            Some(index) => Screen::Layers(index),
            None => Screen::List,
        };

        vec![Action::StacksLoaded(selected_index), Action::ClearStatus]
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

        let mut actions: Vec<Action> = self
            .stack_browser
            .handle_key(key, &self.state)
            .into_iter()
            .map(wrap_quit_in_confirmation)
            .collect();

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

            if matches!(action, Action::SelectNext | Action::SelectPrevious)
                && let Screen::Layers(stack_index) = self.state.screen
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

    /// The layers to submit for `stack_index`, or `None` when the stack has
    /// none (already vanished, or has no layers to push).
    fn submit_layers(&self, stack_index: usize) -> Option<Vec<Layer>> {
        let stack = self.state.stacks.get(stack_index)?;
        if stack.layers.is_empty() {
            return None;
        }
        Some(stack.layers.clone())
    }

    /// The branch `gh stack checkout` should target: the given layer, or
    /// for a whole stack its current layer (falling back to the top one).
    fn checkout_target(
        &mut self,
        stack_index: usize,
        layer_index: Option<usize>,
    ) -> Option<String> {
        let Some(stack) = self.state.stacks.get(stack_index) else {
            self.state.status = Some("selected stack is no longer available".to_string());
            return None;
        };

        let layer = match layer_index {
            Some(index) => stack.layers.get(index).or_else(|| {
                self.state.status = Some("selected layer is no longer available".to_string());
                None
            }),
            None => stack
                .layers
                .iter()
                .find(|layer| layer.is_current)
                .or_else(|| stack.layers.last())
                .or_else(|| {
                    self.state.status = Some("selected stack has no layers".to_string());
                    None
                }),
        }?;

        Some(layer.branch.clone())
    }

    /// The reducer: updates state for `action`, hands any process work to
    /// `effects`, and returns follow-up actions to apply immediately.
    fn apply_action(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::Quit => {
                self.state.should_quit = true;
                Vec::new()
            }
            Action::RefreshStacks => {
                self.state.refresh_request_id += 1;
                vec![Action::StackRefreshStarted {
                    request_id: self.state.refresh_request_id,
                }]
            }
            Action::StackRefreshStarted { request_id } => {
                self.state.refresh_in_flight = true;
                self.state.refresh_spinner_frame = 0;
                self.state.refresh_active_request_id = Some(*request_id);
                if !self.state.stacks.is_empty() {
                    self.state.last_successful_stacks = self.state.stacks.clone();
                }
                effects.load_stacks(*request_id);
                Vec::new()
            }
            Action::StackRefreshSucceeded { request_id, result } => {
                if Some(*request_id) != self.state.refresh_active_request_id {
                    return Vec::new();
                }

                self.state.refresh_in_flight = false;
                self.state.refresh_active_request_id = None;

                match result {
                    Ok(stacks) => self.apply_stacks_loaded(stacks.clone()),
                    Err(error) => vec![Action::SetError(format!(
                        "failed to refresh stacks: {}",
                        friendly_stack_refresh_error(error)
                    ))],
                }
            }
            Action::Tick => {
                if self.state.refresh_in_flight {
                    self.state.refresh_spinner_frame =
                        self.state.refresh_spinner_frame.wrapping_add(1);
                }
                Vec::new()
            }
            Action::ShowLayers(index) => {
                if *index < self.state.stacks.len() {
                    self.state.screen = Screen::Layers(*index);
                    vec![
                        Action::LoadLayerDetail {
                            stack_index: *index,
                            layer_index: 0,
                            force: false,
                        },
                        Action::LoadLayerDiff {
                            stack_index: *index,
                            layer_index: 0,
                            force: false,
                        },
                    ]
                } else {
                    self.state.screen = Screen::List;
                    self.state.status = Some("selected stack is no longer available".to_string());
                    Vec::new()
                }
            }
            Action::CheckoutSelected {
                stack_index,
                layer_index,
            } => {
                if let Some(branch) = self.checkout_target(*stack_index, *layer_index) {
                    effects.checkout(branch);
                }
                Vec::new()
            }
            Action::AddLayer {
                stack_index,
                branch,
                message,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                // `gh stack add` operates on whatever stack is currently
                // checked out; refuse rather than silently mutating a
                // different stack than the one the user was browsing.
                if !stack.is_current {
                    return vec![Action::SetError(
                        "checkout this stack before adding a layer".to_string(),
                    )];
                }

                effects.add_layer(branch.clone(), message.clone());
                Vec::new()
            }
            Action::OpenPullRequest {
                stack_index,
                layer_index,
            } => {
                match self
                    .state
                    .stacks
                    .get(*stack_index)
                    .and_then(|stack| stack.layers.get(*layer_index))
                    .and_then(|layer| layer.pull_request.as_ref())
                {
                    Some(pr) => effects.open_pull_request(pr.number),
                    None => {
                        self.state.status = Some("selected layer has no pull request".to_string())
                    }
                }
                Vec::new()
            }
            Action::LoadLayerDetail {
                stack_index,
                layer_index,
                force,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };
                let Some(layer) = stack.layers.get(*layer_index) else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return Vec::new();
                };

                let cache_key = layer_detail_cache_key(stack, layer);
                if self.state.layer_details.should_load(&cache_key, *force) {
                    let layer = layer.clone();
                    self.state.layer_details.mark_loading(cache_key.clone());
                    effects.load_detail(cache_key, layer);
                }
                Vec::new()
            }
            Action::LayerDetailLoaded { cache_key, result } => {
                self.state
                    .layer_details
                    .store_result(cache_key.clone(), result.clone());
                match result {
                    Err(error) => vec![Action::SetError(error.clone())],
                    Ok(_) => Vec::new(),
                }
            }
            Action::LoadLayerDiff {
                stack_index,
                layer_index,
                force,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };
                let Some(layer) = stack.layers.get(*layer_index) else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return Vec::new();
                };

                let cache_key = layer_diff_cache_key(stack, layer);
                if self.state.layer_diffs.should_load(&cache_key, *force) {
                    let lower = lower_layer_ref(stack, *layer_index);
                    let branch = layer.branch.clone();
                    self.state.layer_diffs.mark_loading(cache_key.clone());
                    effects.load_diff(cache_key, lower, branch);
                }
                Vec::new()
            }
            Action::LayerDiffLoaded { cache_key, result } => {
                self.state
                    .layer_diffs
                    .store_result(cache_key.clone(), result.clone());
                match result {
                    Err(error) => vec![Action::SetError(error.clone())],
                    Ok(_) => Vec::new(),
                }
            }
            Action::ClearStatus => {
                self.state.status = None;
                Vec::new()
            }
            Action::StacksLoaded(_) => {
                if let Screen::Layers(index) = self.state.screen
                    && index >= self.state.stacks.len()
                {
                    self.state.screen = Screen::List;
                    self.state.status = Some("selected stack is no longer available".to_string());
                }
                Vec::new()
            }
            Action::SetError(message) => {
                self.state.error = Some(message.clone());
                Vec::new()
            }
            Action::ClearError => {
                self.state.error = None;
                Vec::new()
            }
            Action::ShowConfirm(modal) => {
                self.state.confirm = Some(modal.clone());
                Vec::new()
            }
            Action::ConfirmCancel => {
                self.state.confirm = None;
                Vec::new()
            }
            Action::ConfirmAccept => match self.state.confirm.take() {
                Some(modal) if modal.can_confirm() => {
                    modal.into_confirmed_action().into_iter().collect()
                }
                Some(modal) => {
                    // Danger modal, phrase not typed correctly yet: keep it
                    // open rather than firing or dismissing it.
                    self.state.confirm = Some(modal);
                    Vec::new()
                }
                None => Vec::new(),
            },
            Action::ConfirmInput(c) => {
                if let Some(modal) = self.state.confirm.as_mut() {
                    modal.push_char(*c);
                }
                Vec::new()
            }
            Action::ConfirmBackspace => {
                if let Some(modal) = self.state.confirm.as_mut() {
                    modal.pop_char();
                }
                Vec::new()
            }
            Action::SubmitStack { stack_index } => {
                if let Some(progress) = &self.state.submit_progress
                    && !progress.finished
                {
                    self.state.status = Some("a submit is already in progress".to_string());
                    return Vec::new();
                }

                if self.submit_layers(*stack_index).is_none() {
                    self.state.status = Some("selected stack has no layers to submit".to_string());
                    return Vec::new();
                }

                vec![Action::SubmitStarted {
                    stack_index: *stack_index,
                }]
            }
            Action::SubmitStarted { stack_index } => {
                let Some(layers) = self.submit_layers(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                let branches = layers.iter().map(|layer| layer.branch.clone()).collect();
                self.state.submit_progress = Some(SubmitProgress::new(*stack_index, branches));
                effects.submit_stack(
                    *stack_index,
                    layers,
                    crate::config::get().default_remote.clone(),
                    self.state.submit_options,
                );
                Vec::new()
            }
            Action::SubmitLayerProgress {
                stack_index,
                layer_index,
                status,
            } => {
                if let Some(progress) = &mut self.state.submit_progress
                    && progress.stack_index == *stack_index
                {
                    progress.set_status(*layer_index, status.clone());
                }
                Vec::new()
            }
            Action::SubmitFinished { stack_index } => {
                if let Some(progress) = &mut self.state.submit_progress
                    && progress.stack_index == *stack_index
                {
                    progress.finished = true;
                    return vec![Action::RefreshStacks];
                }
                Vec::new()
            }
            Action::ToggleSubmitAuto => {
                self.state.submit_options.auto = !self.state.submit_options.auto;
                Vec::new()
            }
            Action::ToggleSubmitOpen => {
                self.state.submit_options.open = !self.state.submit_options.open;
                Vec::new()
            }
            Action::DismissSubmit => {
                self.state.submit_progress = None;
                Vec::new()
            }
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
    app.dispatch(vec![Action::RefreshStacks], &effects);

    while !app.state.should_quit {
        while let Ok(action) = rx.try_recv() {
            app.dispatch(vec![action], &effects);
        }

        app.dispatch(vec![Action::Tick], &effects);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellError};
    use crate::stack::StackSummary;
    use crate::test_fixtures::{layer, stack_summary};
    use crate::tui::state::submit_progress::LayerSubmitStatus;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn test_effects(shell: &MockShell) -> (Effects, mpsc::UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let repo = std::env::current_dir().unwrap();
        (Effects::new(repo, Arc::new(shell.clone()), tx), rx)
    }

    impl App {
        /// Applies `action` without letting any effect it spawns run: the
        /// single-threaded test runtime never polls the spawned task.
        fn apply(&mut self, action: &Action) -> Vec<Action> {
            let (effects, _rx) = test_effects(&MockShell::new());
            self.apply_action(action, &effects)
        }

        /// Applies `action`, runs its effects against `shell` to completion,
        /// and returns its immediate follow-ups followed by every action the
        /// effects sent back (neither is applied).
        async fn settle(&mut self, action: &Action, shell: &MockShell) -> Vec<Action> {
            let (effects, mut rx) = test_effects(shell);
            let mut actions = self.apply_action(action, &effects);
            effects.wait_idle().await;
            while let Ok(action) = rx.try_recv() {
                actions.push(action);
            }
            actions
        }

        /// Dispatches `actions` and their follow-ups, without running effects.
        fn dispatch_now(&mut self, actions: Vec<Action>) {
            let (effects, _rx) = test_effects(&MockShell::new());
            self.dispatch(actions, &effects);
        }
    }

    /// Submits every layer of `stack` through the real reducer and effect
    /// runner, returning each layer's final status.
    async fn submit_through_effects(
        stack: StackSummary,
        shell: &MockShell,
    ) -> Vec<LayerSubmitStatus> {
        let mut app = App::new();
        app.state.stacks = vec![stack];

        for action in app
            .settle(&Action::SubmitStarted { stack_index: 0 }, shell)
            .await
        {
            app.apply(&action);
        }

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert!(progress.finished);
        progress
            .layers
            .iter()
            .map(|layer| layer.status.clone())
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn layer_detail_cache_key_includes_stack_and_branch() {
        let stack = stack_summary("stack-a", 1);
        let layer = layer("feature/layer-1");

        assert_eq!(
            layer_detail_cache_key(&stack, &layer),
            "stack-a::feature/layer-1"
        );
    }

    #[test]
    fn layer_diff_cache_key_includes_stack_and_branch() {
        let stack = stack_summary("stack-a", 1);
        let layer = layer("feature/layer-1");

        assert_eq!(
            layer_diff_cache_key(&stack, &layer),
            "stack-a::feature/layer-1::diff"
        );
    }

    #[test]
    fn lower_layer_ref_uses_trunk_for_bottom_layer() {
        let stack = stack_summary("stack-a", 2);

        assert_eq!(lower_layer_ref(&stack, 0), "main");
    }

    #[test]
    fn lower_layer_ref_uses_previous_layer_for_higher_layers() {
        let stack = stack_summary("stack-a", 2);

        assert_eq!(lower_layer_ref(&stack, 1), "stack-a-layer-0");
    }

    #[tokio::test]
    async fn show_layers_loads_first_layer_detail() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app.settle(&Action::ShowLayers(0), &shell).await;

        assert!(matches!(app.state.screen, Screen::Layers(0)));
        assert!(matches!(
            follow_ups.as_slice(),
            [
                Action::LoadLayerDetail {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                },
                Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                }
            ]
        ));
    }

    #[tokio::test]
    async fn checkout_selected_stack_uses_current_layer_when_present() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-0"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];
        app.state.stacks[0].layers[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_stack_uses_top_layer_when_none_current() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-1"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_layer_uses_selected_layer_branch() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-1"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: Some(1),
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_returns_set_error_on_checkout_failure() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-0"],
            Err(crate::shell::ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_with_message() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer", "-m", "add new layer"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: Some("add new layer".to_string()),
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_without_message_flag() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_returns_set_error_on_failure() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer"],
            Err(crate::shell::ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn add_layer_refuses_to_run_on_a_non_current_stack() {
        // No responses registered: the shell must not be invoked at all.
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    /// A failed load used to raise the error without recording it, leaving
    /// the cache entry stuck in `Loading` ("Loading diff..." forever).
    #[tokio::test]
    async fn failed_layer_diff_load_stops_loading_and_reports_error() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        let key = layer_diff_cache_key(&app.state.stacks[0], &app.state.stacks[0].layers[0]);
        app.state.layer_diffs.mark_loading(key.clone());

        let follow_ups = app.apply(&Action::LayerDiffLoaded {
            cache_key: key.clone(),
            result: Err("load layer diff: boom".to_string()),
        });

        assert!(!app.state.layer_diffs.is_loading(&key));
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn load_layer_diff_diffs_bottom_layer_against_trunk() {
        let shell = MockShell::new().when(
            "git",
            &[
                "diff",
                "--color=never",
                "--find-renames",
                "main..stack-a-layer-0",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "+bottom".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let results = app
            .settle(
                &Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                },
                &shell,
            )
            .await;
        for action in &results {
            app.apply(action);
        }

        let key = layer_diff_cache_key(&app.state.stacks[0], &app.state.stacks[0].layers[0]);
        assert_eq!(app.state.layer_diffs.get(&key).unwrap(), "+bottom");
    }

    #[tokio::test]
    async fn refresh_stacks_creates_started_action_without_blocking() {
        let shell = MockShell::new();
        let mut app = App::new();

        let follow_ups = app.settle(&Action::RefreshStacks, &shell).await;

        assert!(matches!(
            follow_ups.as_slice(),
            [Action::StackRefreshStarted { request_id: 1 }]
        ));
        assert!(!app.state.refresh_in_flight);
    }

    #[tokio::test]
    async fn refresh_started_sets_loading_and_keeps_existing_stacks() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        app.apply(&Action::StackRefreshStarted { request_id: 9 });

        assert!(app.state.refresh_in_flight);
        assert_eq!(app.state.refresh_active_request_id, Some(9));
        assert_eq!(app.state.stacks.len(), 1);
        assert_eq!(app.state.last_successful_stacks.len(), 1);
    }

    #[tokio::test]
    async fn refresh_success_ignores_outdated_request_ids() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.refresh_active_request_id = Some(2);

        app.settle(
            &Action::StackRefreshSucceeded {
                request_id: 1,
                result: Ok(vec![stack_summary("stack-b", 1)]),
            },
            &shell,
        )
        .await;

        assert_eq!(app.state.stacks[0].label, "stack-a");
    }

    #[tokio::test]
    async fn refresh_failure_sets_dismissible_error() {
        let mut app = App::new();
        app.state.refresh_active_request_id = Some(4);
        app.state.refresh_in_flight = true;

        app.dispatch_now(vec![Action::StackRefreshSucceeded {
            request_id: 4,
            result: Err("network timeout".to_string()),
        }]);

        assert!(!app.state.refresh_in_flight);
        assert!(app.state.error.is_some());
    }

    #[tokio::test]
    async fn tick_advances_spinner_while_refreshing() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.refresh_in_flight = true;

        app.settle(&Action::Tick, &shell).await;
        assert_eq!(app.state.refresh_spinner_frame, 1);
    }

    // --- Confirm modal framework: end-to-end via the real key-handling and
    // apply_action code paths, not a hand-rolled bypass of them. `Quit` is
    // the one real call site wired up today (see `wrap_quit_in_confirmation`
    // doc comment for why); the `danger` flow below is exercised directly
    // against a modal built the same way a future sync/rebase/merge/
    // restructure/unstack call site would build one, since none of those
    // actions exist yet.

    #[tokio::test]
    async fn quit_key_shows_confirmation_instead_of_quitting_immediately() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let actions = app.handle_key(key(KeyCode::Char('q')));
        assert!(matches!(actions.as_slice(), [Action::ShowConfirm(_)]));

        app.dispatch_now(actions);

        let modal = app.state.confirm.as_ref().expect("modal should be shown");
        assert!(!modal.danger);
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn confirming_quit_modal_actually_quits() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_now(show);
        assert!(app.state.confirm.is_some());
        assert!(!app.state.should_quit);

        let accept = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(accept.as_slice(), [Action::ConfirmAccept]));
        app.dispatch_now(accept);

        assert!(app.state.should_quit);
        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn cancelling_quit_modal_leaves_app_running() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_now(show);

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_now(cancel);

        assert!(app.state.confirm.is_none());
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn danger_modal_requires_typed_phrase_before_enter_fires_action() {
        let mut app = App::new();

        app.state.confirm = Some(
            ConfirmModal::new(
                "Merge PR?",
                "This merges the stack into main.",
                "Merge",
                true,
            )
            .on_confirm(Action::ClearStatus),
        );

        // Enter does nothing yet: no phrase has been typed at all.
        let premature = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(premature);
        assert!(app.state.confirm.is_some());

        // Typing the wrong phrase also keeps it open.
        for c in "no".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }
        let premature = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(premature);
        assert!(app.state.confirm.is_some());

        for _ in 0.."no".len() {
            let backspace = app.handle_key(key(KeyCode::Backspace));
            app.dispatch_now(backspace);
        }

        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }
        assert_eq!(app.state.confirm.as_ref().unwrap().typed_input(), "yes");

        let accept = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(accept);

        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn danger_modal_esc_cancels_without_firing_action() {
        let mut app = App::new();
        app.state.status = Some("untouched".to_string());

        app.state.confirm = Some(
            ConfirmModal::new(
                "Merge PR?",
                "This merges the stack into main.",
                "Merge",
                true,
            )
            .on_confirm(Action::ClearStatus),
        );

        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_now(cancel);

        assert!(app.state.confirm.is_none());
        // The attached action (ClearStatus) must never have fired.
        assert_eq!(app.state.status.as_deref(), Some("untouched"));
    }

    fn push_ok(
        branch: &str,
    ) -> (
        &'static str,
        Vec<String>,
        Result<crate::shell::ShellOutput, ShellError>,
    ) {
        (
            "git",
            vec![
                "push".to_string(),
                "--force-with-lease".to_string(),
                "--set-upstream".to_string(),
                "origin".to_string(),
                format!("{branch}:{branch}"),
            ],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        )
    }

    fn mock_when(
        shell: MockShell,
        call: (
            &'static str,
            Vec<String>,
            Result<crate::shell::ShellOutput, ShellError>,
        ),
    ) -> MockShell {
        let (program, args, result) = call;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        shell.when(program, &args, result)
    }

    #[tokio::test]
    async fn submit_stack_action_starts_tracking_progress_for_every_layer() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(&Action::SubmitStack { stack_index: 0 }, &shell)
            .await;
        assert!(matches!(
            follow_ups.as_slice(),
            [Action::SubmitStarted { stack_index: 0 }]
        ));

        app.apply(&follow_ups[0]);

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert_eq!(progress.total(), 2);
        assert_eq!(progress.completed_count(), 0);
        assert!(!progress.finished);
    }

    #[tokio::test]
    async fn submit_stack_action_refuses_to_start_a_second_run_while_one_is_in_progress() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.submit_progress =
            Some(SubmitProgress::new(0, vec!["stack-a-layer-0".to_string()]));

        let follow_ups = app
            .settle(&Action::SubmitStack { stack_index: 0 }, &shell)
            .await;

        assert!(follow_ups.is_empty());
        assert!(app.state.status.is_some());
    }

    #[tokio::test]
    async fn submit_layer_progress_action_updates_only_the_matching_layer() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(
            0,
            vec!["a".to_string(), "b".to_string()],
        ));

        app.settle(
            &Action::SubmitLayerProgress {
                stack_index: 0,
                layer_index: 1,
                status: LayerSubmitStatus::Failed("boom".to_string()),
            },
            &shell,
        )
        .await;

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert_eq!(progress.layers[0].status, LayerSubmitStatus::Pending);
        assert_eq!(
            progress.layers[1].status,
            LayerSubmitStatus::Failed("boom".to_string())
        );
    }

    #[tokio::test]
    async fn submit_finished_action_marks_progress_finished_and_refreshes_stacks() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        let follow_ups = app
            .settle(&Action::SubmitFinished { stack_index: 0 }, &shell)
            .await;

        assert!(app.state.submit_progress.as_ref().unwrap().finished);
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn toggle_actions_flip_submit_options() {
        let shell = MockShell::new();
        let mut app = App::new();
        assert!(!app.state.submit_options.auto);
        assert!(!app.state.submit_options.open);

        app.settle(&Action::ToggleSubmitAuto, &shell).await;
        app.settle(&Action::ToggleSubmitOpen, &shell).await;

        assert!(app.state.submit_options.auto);
        assert!(app.state.submit_options.open);
    }

    #[tokio::test]
    async fn dismiss_submit_clears_progress() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        app.settle(&Action::DismissSubmit, &shell).await;

        assert!(app.state.submit_progress.is_none());
    }

    /// Reproduces the issue's acceptance criterion directly: layer 2 of 4
    /// fails to push while the other three succeed, and the failure must be
    /// visible on that layer alone rather than failing the whole submit.
    #[tokio::test]
    async fn submit_through_effects_reports_a_partial_failure_per_layer() {
        let stack = stack_summary("stack-a", 4);

        let mut shell = MockShell::new();
        shell = mock_when(shell, push_ok("stack-a-layer-0"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-0",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/1".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        shell = shell.when(
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                "origin",
                "stack-a-layer-1:stack-a-layer-1",
            ],
            Err(ShellError::CommandFailed {
                program: "git".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "stale info".to_string(),
                    exit_code: 1,
                },
            }),
        );
        shell = mock_when(shell, push_ok("stack-a-layer-2"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-2",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/3".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        shell = mock_when(shell, push_ok("stack-a-layer-3"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-3",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/4".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let statuses = submit_through_effects(stack, &shell).await;

        assert_eq!(statuses.len(), 4);
        assert_eq!(
            statuses[0],
            LayerSubmitStatus::PullRequestCreated { number: 1 }
        );
        assert!(matches!(&statuses[1], LayerSubmitStatus::Failed(_)));
        assert_eq!(
            statuses[2],
            LayerSubmitStatus::PullRequestCreated { number: 3 }
        );
        assert_eq!(
            statuses[3],
            LayerSubmitStatus::PullRequestCreated { number: 4 }
        );
    }

    #[tokio::test]
    async fn submit_through_effects_reports_every_layer_succeeding() {
        let stack = stack_summary("stack-a", 2);

        let mut shell = MockShell::new();
        for (index, branch) in ["stack-a-layer-0", "stack-a-layer-1"].iter().enumerate() {
            shell = mock_when(shell, push_ok(branch));
            shell = shell.when(
                "gh",
                &["pr", "create", "--head", branch, "--base", "main", "--fill"],
                Ok(crate::shell::ShellOutput {
                    stdout: format!("https://github.com/o/r/pull/{}", index + 1),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            );
        }

        let statuses = submit_through_effects(stack, &shell).await;

        assert_eq!(
            statuses,
            vec![
                LayerSubmitStatus::PullRequestCreated { number: 1 },
                LayerSubmitStatus::PullRequestCreated { number: 2 },
            ]
        );
    }
}
