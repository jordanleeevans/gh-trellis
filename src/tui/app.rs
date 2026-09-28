use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::git;
use crate::shell::{ProcessShell, Shell, ShellError};
use crate::stack::hydrate_layer_detail;
use crate::stack::{
    Layer, LayerDetail, StackSummary, SubmitOptions, list_stacks, push_layer_branch,
    sync_layer_pull_request,
};

use super::confirm::{self, ConfirmModal};
use super::keymap::{KeyIntent, key_intent};
use super::layer_resource::LayerResourceCache;
use super::stack_layers;
use super::submit_progress::{LayerSubmitStatus, SubmitProgress};

/// Which stack is currently selected in the unified browser.
#[derive(Debug, Clone, Copy)]
pub enum Screen {
    /// No stack is currently selected.
    List,
    /// The unified browser focused on the stack at this index into [`AppState::stacks`].
    Layers(usize),
}

#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    RefreshStacks,
    SelectNext,
    SelectPrevious,
    FocusNextPanel,
    FocusPreviousPanel,
    SelectNextDiffFile,
    SelectPreviousDiffFile,
    ScrollDiffLineDown,
    ScrollDiffLineUp,
    ScrollDiffDown,
    ScrollDiffUp,
    ScrollDiffHalfPageDown,
    ScrollDiffHalfPageUp,
    ScrollDiffTop,
    ScrollDiffBottom,
    ToggleDiffView,
    ShowLayers(usize),
    CheckoutSelected {
        stack_index: usize,
        layer_index: Option<usize>,
    },
    AddLayer {
        stack_index: usize,
        branch: String,
        message: Option<String>,
    },
    OpenPullRequest {
        stack_index: usize,
        layer_index: usize,
    },
    LoadLayerDetail {
        stack_index: usize,
        layer_index: usize,
        force: bool,
    },
    LoadLayerDiff {
        stack_index: usize,
        layer_index: usize,
        force: bool,
    },
    LayerDetailLoaded {
        cache_key: String,
        result: Result<LayerDetail, String>,
    },
    LayerDiffLoaded {
        cache_key: String,
        result: Result<String, String>,
    },
    StackRefreshStarted {
        request_id: u64,
    },
    StackRefreshSucceeded {
        request_id: u64,
        result: Result<Vec<StackSummary>, String>,
    },
    Tick,
    StacksLoaded(Option<usize>),
    SetError(String),
    ClearError,
    ClearStatus,
    /// Shows a confirmation modal (see [`crate::tui::confirm::ConfirmModal`]),
    /// replacing any modal already shown.
    ShowConfirm(ConfirmModal),
    /// Accepts the currently shown confirm modal, dispatching its attached
    /// action if [`ConfirmModal::can_confirm`] allows it, or leaving it open
    /// (waiting for the danger phrase to be completed) otherwise.
    ConfirmAccept,
    /// Dismisses the currently shown confirm modal without dispatching its
    /// attached action.
    ConfirmCancel,
    /// Appends a typed character to a `danger` confirm modal's input.
    ConfirmInput(char),
    /// Removes the last typed character from a `danger` confirm modal's
    /// input.
    ConfirmBackspace,
    SubmitStack {
        stack_index: usize,
    },
    ToggleSubmitAuto,
    ToggleSubmitOpen,
    SubmitStarted {
        stack_index: usize,
    },
    SubmitLayerProgress {
        stack_index: usize,
        layer_index: usize,
        status: LayerSubmitStatus,
    },
    SubmitFinished {
        stack_index: usize,
    },
    DismissSubmit,
}

pub trait Component {
    fn draw(&mut self, frame: &mut Frame, state: &AppState);
    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action>;
    fn update(&mut self, action: &Action, state: &mut AppState);
}

pub struct AppState {
    pub stacks: Vec<StackSummary>,
    pub screen: Screen,
    pub status: Option<String>,
    pub error: Option<String>,
    pub refresh_in_flight: bool,
    pub refresh_spinner_frame: usize,
    pub refresh_request_id: u64,
    pub refresh_active_request_id: Option<u64>,
    pub last_successful_stacks: Vec<StackSummary>,
    pub layer_details: LayerResourceCache<LayerDetail>,
    pub layer_diffs: LayerResourceCache<String>,
    /// The confirmation modal currently shown on top of whatever screen is
    /// active, if any. Any destructive (or otherwise confirmation-worthy)
    /// action shows one via `Action::ShowConfirm` rather than rolling its
    /// own one-off modal.
    pub confirm: Option<ConfirmModal>,
    pub submit_progress: Option<SubmitProgress>,
    pub submit_options: SubmitOptions,
    pub(crate) should_quit: bool,
}

struct App {
    state: AppState,
    stack_layers: stack_layers::StackLayers,
}

impl AppState {
    fn new() -> Self {
        Self {
            stacks: Vec::new(),
            screen: Screen::List,
            status: None,
            error: None,
            refresh_in_flight: false,
            refresh_spinner_frame: 0,
            refresh_request_id: 0,
            refresh_active_request_id: None,
            last_successful_stacks: Vec::new(),
            layer_details: LayerResourceCache::default(),
            layer_diffs: LayerResourceCache::default(),
            confirm: None,
            submit_progress: None,
            submit_options: SubmitOptions::default(),
            should_quit: false,
        }
    }
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::new(),
            stack_layers: stack_layers::StackLayers::new(),
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
        let mut actions = vec![Action::StacksLoaded(selected_index)];

        if let Some(index) = selected_index {
            self.state.screen = Screen::Layers(index);
            actions.push(Action::ClearStatus);
        } else {
            self.state.screen = Screen::List;
            actions.push(Action::ClearStatus);
        }

        actions
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.stack_layers.draw(frame, &self.state);

        if let Some(modal) = &self.state.confirm {
            confirm::render(frame, frame.area(), modal);
        }
    }

    /// Turns a raw key event into `Action`s. A confirm modal, when shown,
    /// gets first refusal on every key: input is routed to it instead of
    /// falling through to the active screen's own key handling, exactly the
    /// way `stack_layers` already intercepts keys internally based on its
    /// own state (e.g. its pending-`g` handling).
    fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if let Some(modal) = &self.state.confirm {
            return confirm::handle_confirm_key(modal, key);
        }

        let mut actions: Vec<Action> = self
            .stack_layers
            .handle_key(key, &self.state)
            .into_iter()
            .map(wrap_quit_in_confirmation)
            .collect();

        if self.state.error.is_some() && key_intent(key) == Some(KeyIntent::DismissMessage) {
            actions.push(Action::ClearError);
        }

        actions
    }

    async fn dispatch_actions_with_loader(
        &mut self,
        actions: Vec<Action>,
        shell: &impl Shell,
        repo: &Path,
        loader: Option<&ActionScheduler>,
    ) {
        let mut pending = std::collections::VecDeque::from(actions);

        while let Some(action) = pending.pop_front() {
            let follow_ups = if let Some(loader) = loader {
                if self.schedule_async_action(&action, loader) {
                    Vec::new()
                } else {
                    self.apply_action(&action, shell, repo).await
                }
            } else {
                self.apply_action(&action, shell, repo).await
            };
            self.stack_layers.update(&action, &mut self.state);
            pending.extend(follow_ups);

            if matches!(action, Action::SelectNext | Action::SelectPrevious)
                && let Screen::Layers(stack_index) = self.state.screen
                && let Some(layer_index) = self.stack_layers.selected_index()
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

    fn schedule_async_action(&mut self, action: &Action, loader: &ActionScheduler) -> bool {
        match action {
            Action::StackRefreshStarted { request_id } => {
                self.state.refresh_in_flight = true;
                self.state.refresh_spinner_frame = 0;
                self.state.refresh_active_request_id = Some(*request_id);
                if !self.state.stacks.is_empty() {
                    self.state.last_successful_stacks = self.state.stacks.clone();
                }
                loader.load_stacks(*request_id);
                true
            }
            Action::LoadLayerDetail {
                stack_index,
                layer_index,
                force,
            } => {
                let Some((cache_key, layer)) = self.layer_load_context(*stack_index, *layer_index)
                else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return true;
                };

                if !self.state.layer_details.should_load(&cache_key, *force) {
                    return true;
                }

                self.state.layer_details.mark_loading(cache_key.clone());
                loader.load_detail(cache_key, layer);
                true
            }
            Action::LoadLayerDiff {
                stack_index,
                layer_index,
                force,
            } => {
                let Some((cache_key, branch, lower)) =
                    self.layer_diff_context(*stack_index, *layer_index)
                else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return true;
                };

                if !self.state.layer_diffs.should_load(&cache_key, *force) {
                    return true;
                }

                self.state.layer_diffs.mark_loading(cache_key.clone());
                loader.load_diff(cache_key, lower, branch);
                true
            }
            Action::SubmitStarted { stack_index } => {
                let Some(layers) = self.submit_layers(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return true;
                };

                let branches = layers.iter().map(|layer| layer.branch.clone()).collect();
                self.state.submit_progress = Some(SubmitProgress::new(*stack_index, branches));
                loader.submit_stack(*stack_index, layers, self.state.submit_options);
                true
            }
            _ => false,
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

    fn layer_load_context(
        &self,
        stack_index: usize,
        layer_index: usize,
    ) -> Option<(String, Layer)> {
        let stack = self.state.stacks.get(stack_index)?;
        let layer = stack.layers.get(layer_index)?;
        Some((layer_detail_cache_key(stack, layer), layer.clone()))
    }

    fn layer_diff_context(
        &self,
        stack_index: usize,
        layer_index: usize,
    ) -> Option<(String, String, String)> {
        let stack = self.state.stacks.get(stack_index)?;
        let layer = stack.layers.get(layer_index)?;
        Some((
            layer_diff_cache_key(stack, layer),
            layer.branch.clone(),
            lower_layer_ref(stack, layer_index),
        ))
    }

    async fn apply_action(
        &mut self,
        action: &Action,
        shell: &impl Shell,
        repo: &Path,
    ) -> Vec<Action> {
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
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                let target_branch = match layer_index {
                    Some(index) => {
                        let Some(layer) = stack.layers.get(*index) else {
                            self.state.status =
                                Some("selected layer is no longer available".to_string());
                            return Vec::new();
                        };
                        layer.branch.clone()
                    }
                    None => {
                        let Some(layer) = stack
                            .layers
                            .iter()
                            .find(|layer| layer.is_current)
                            .or_else(|| stack.layers.last())
                        else {
                            self.state.status = Some("selected stack has no layers".to_string());
                            return Vec::new();
                        };
                        layer.branch.clone()
                    }
                };

                if let Err(error) = shell
                    .run(repo, "gh", &["stack", "checkout", &target_branch])
                    .await
                {
                    return vec![Action::SetError(friendly_shell_error(
                        "checkout stack",
                        &error,
                    ))];
                }
                vec![Action::RefreshStacks]
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

                let mut args = vec!["stack", "add", branch.as_str()];
                if let Some(message) = message {
                    args.push("-m");
                    args.push(message.as_str());
                }

                if let Err(error) = shell.run(repo, "gh", &args).await {
                    return vec![Action::SetError(friendly_shell_error("add layer", &error))];
                }
                vec![Action::RefreshStacks]
            }
            Action::OpenPullRequest {
                stack_index,
                layer_index,
            } => {
                if let Some(pr) = self
                    .state
                    .stacks
                    .get(*stack_index)
                    .and_then(|stack| stack.layers.get(*layer_index))
                    .and_then(|layer| layer.pull_request.as_ref())
                {
                    let pr_number = pr.number.to_string();
                    if let Err(error) = shell
                        .run(repo, "gh", &["pr", "view", &pr_number, "--web"])
                        .await
                    {
                        return vec![Action::SetError(friendly_shell_error(
                            "open pull request",
                            &error,
                        ))];
                    }
                } else {
                    self.state.status = Some("selected layer has no pull request".to_string());
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
                if !self.state.layer_details.should_load(&cache_key, *force) {
                    return Vec::new();
                }

                match hydrate_layer_detail(shell, repo, layer).await {
                    Ok(detail) => {
                        self.state.layer_details.store_result(cache_key, Ok(detail));
                    }
                    Err(error) => {
                        let message = friendly_shell_error("load layer detail", &error);
                        self.state
                            .layer_details
                            .store_result(cache_key, Err(message.clone()));
                        return vec![Action::SetError(message)];
                    }
                }

                Vec::new()
            }
            Action::LayerDetailLoaded { cache_key, result } => {
                if let Err(error) = result {
                    return vec![Action::SetError(error.clone())];
                }
                self.state
                    .layer_details
                    .store_result(cache_key.clone(), result.clone());
                Vec::new()
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
                if !self.state.layer_diffs.should_load(&cache_key, *force) {
                    return Vec::new();
                }

                let lower = lower_layer_ref(stack, *layer_index);
                match git::diff(shell, repo, &lower, &layer.branch).await {
                    Ok(diff) => {
                        self.state.layer_diffs.store_result(cache_key, Ok(diff));
                    }
                    Err(error) => {
                        let message = friendly_shell_error("load layer diff", &error);
                        self.state
                            .layer_diffs
                            .store_result(cache_key, Err(message.clone()));
                        return vec![Action::SetError(message)];
                    }
                }

                Vec::new()
            }
            Action::LayerDiffLoaded { cache_key, result } => {
                if let Err(error) = result {
                    return vec![Action::SetError(error.clone())];
                }
                self.state
                    .layer_diffs
                    .store_result(cache_key.clone(), result.clone());
                Vec::new()
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
/// handling, e.g. `q`/Esc in `stack_layers`) in a non-danger confirm modal
/// instead of letting it fire immediately, so accidentally hitting `q`
/// doesn't kill the whole TUI. This is the framework's one real, wired-up
/// demonstration: `Action::Quit` itself is untouched and still performs the
/// actual quit once the modal is confirmed, since this rewrite only happens
/// at the key-handling boundary, not inside `apply_action`'s reducer, so
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

struct ActionScheduler {
    repo: PathBuf,
    tx: mpsc::UnboundedSender<Action>,
}

impl ActionScheduler {
    fn new(repo: PathBuf, tx: mpsc::UnboundedSender<Action>) -> Self {
        Self { repo, tx }
    }

    fn load_stacks(&self, request_id: u64) {
        let repo = self.repo.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = list_stacks(&ProcessShell, repo.as_path())
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(Action::StackRefreshSucceeded { request_id, result });
        });
    }

    fn load_detail(&self, cache_key: String, layer: Layer) {
        let repo = self.repo.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = hydrate_layer_detail(&ProcessShell, repo.as_path(), &layer)
                .await
                .map_err(|error| friendly_shell_error("load layer detail", &error));
            let _ = tx.send(Action::LayerDetailLoaded { cache_key, result });
        });
    }

    fn load_diff(&self, cache_key: String, lower: String, branch: String) {
        let repo = self.repo.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = git::diff(&ProcessShell, repo.as_path(), &lower, &branch)
                .await
                .map_err(|error| friendly_shell_error("load layer diff", &error));
            let _ = tx.send(Action::LayerDiffLoaded { cache_key, result });
        });
    }

    /// Submits every layer of the stack at `stack_index`, one at a time,
    /// sending a [`Action::SubmitLayerProgress`] as each layer's push
    /// completes and again as its pull request step completes, so a
    /// partial failure on one layer is reported for that layer alone —
    /// processing continues on to the rest of the stack rather than
    /// aborting the whole submit.
    fn submit_stack(&self, stack_index: usize, layers: Vec<Layer>, options: SubmitOptions) {
        let repo = self.repo.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            run_submit_sequence(
                &ProcessShell,
                repo.as_path(),
                &crate::config::get().default_remote,
                &layers,
                options,
                |layer_index, status| {
                    let _ = tx.send(Action::SubmitLayerProgress {
                        stack_index,
                        layer_index,
                        status,
                    });
                },
            )
            .await;
            let _ = tx.send(Action::SubmitFinished { stack_index });
        });
    }
}

/// Pushes and syncs the pull request for each of `layers` in order,
/// reporting one status per layer as it changes via `on_progress`
/// (`layer_index` into `layers`).
///
/// A layer whose push or pull request step fails is reported as
/// [`LayerSubmitStatus::Failed`] and the loop moves on to the next layer —
/// a failure on one layer never stops the rest of the stack from being
/// submitted, and never gets folded into a single overall pass/fail result.
async fn run_submit_sequence(
    shell: &impl Shell,
    repo: &Path,
    remote: &str,
    layers: &[Layer],
    options: SubmitOptions,
    mut on_progress: impl FnMut(usize, LayerSubmitStatus),
) {
    for (layer_index, layer) in layers.iter().enumerate() {
        on_progress(layer_index, LayerSubmitStatus::InProgress);

        if let Err(error) = push_layer_branch(shell, repo, remote, &layer.branch).await {
            on_progress(
                layer_index,
                LayerSubmitStatus::Failed(friendly_shell_error("push layer", &error)),
            );
            continue;
        }

        on_progress(layer_index, LayerSubmitStatus::Pushed);

        let status = match sync_layer_pull_request(shell, repo, layer, &options).await {
            Ok(crate::stack::SubmitLayerOutcome::PullRequestCreated { number }) => {
                LayerSubmitStatus::PullRequestCreated { number }
            }
            Ok(crate::stack::SubmitLayerOutcome::PullRequestUpdated { number }) => {
                LayerSubmitStatus::PullRequestUpdated { number }
            }
            Err(error) => {
                LayerSubmitStatus::Failed(friendly_shell_error("submit pull request", &error))
            }
        };

        on_progress(layer_index, status);
    }
}

pub fn spinner_frame(index: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    FRAMES[index % FRAMES.len()]
}

fn friendly_stack_refresh_error(error: &str) -> String {
    let normalized = error.to_lowercase();

    if normalized.contains("timed out") {
        return "request timed out while refreshing stacks; check network/auth and retry"
            .to_string();
    }
    if normalized.contains("rate limit") {
        return "GitHub API rate limit reached; retry later".to_string();
    }
    if normalized.contains("not logged in") || normalized.contains("authentication") {
        return "GitHub authentication required; run `gh auth login`".to_string();
    }
    if normalized.contains("network") || normalized.contains("could not resolve host") {
        return "network failure while refreshing stacks".to_string();
    }

    error.to_string()
}

fn friendly_shell_error(context: &str, error: &ShellError) -> String {
    match error {
        ShellError::Timeout { .. } => {
            format!("{context}: request timed out; check network/auth and retry")
        }
        ShellError::BinaryNotFound(program) => {
            format!("{context}: required binary `{program}` not found")
        }
        ShellError::CommandFailed { program, output } if program == "gh" => {
            let stderr = output.stderr.to_lowercase();
            if stderr.contains("rate limit") {
                format!("{context}: GitHub API rate limit reached; retry later")
            } else if stderr.contains("not logged in")
                || stderr.contains("authentication")
                || stderr.contains("401")
            {
                format!("{context}: GitHub authentication required; run `gh auth login`")
            } else if stderr.contains("network")
                || stderr.contains("timed out")
                || stderr.contains("could not resolve host")
            {
                format!("{context}: network failure while calling gh")
            } else {
                format!("{context}: {}", output.stderr.trim())
            }
        }
        ShellError::CommandFailed { output, .. } => format!("{context}: {}", output.stderr.trim()),
        _ => format!("{context}: {error}"),
    }
}

/// Runs the TUI until the user quits, then restores the terminal.
pub async fn run(shell: &impl Shell, repo: &Path) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, shell, repo).await;
    ratatui::restore();
    result
}

/// Runs the app
async fn run_app(
    terminal: &mut DefaultTerminal,
    shell: &impl Shell,
    repo: &Path,
) -> anyhow::Result<()> {
    let mut app = App::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let loader = ActionScheduler::new(repo.to_path_buf(), tx);
    app.dispatch_actions_with_loader(vec![Action::RefreshStacks], shell, repo, Some(&loader))
        .await;

    while !app.state.should_quit {
        while let Ok(action) = rx.try_recv() {
            app.dispatch_actions_with_loader(vec![action], shell, repo, Some(&loader))
                .await;
        }

        app.dispatch_actions_with_loader(vec![Action::Tick], shell, repo, Some(&loader))
            .await;

        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let actions = app.handle_key(key);
            app.dispatch_actions_with_loader(actions, shell, repo, Some(&loader))
                .await;
        }
    }

    Ok(())
}

pub fn layer_detail_cache_key(stack: &StackSummary, layer: &Layer) -> String {
    format!("{}::{}", stack.label, layer.branch)
}

pub fn layer_diff_cache_key(stack: &StackSummary, layer: &Layer) -> String {
    format!("{}::{}::diff", stack.label, layer.branch)
}

pub fn lower_layer_ref(stack: &StackSummary, layer_index: usize) -> String {
    stack
        .layers
        .get(layer_index.saturating_sub(1))
        .filter(|_| layer_index > 0)
        .map(|layer| layer.branch.clone())
        .unwrap_or_else(|| stack.trunk.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::MockShell;
    use crate::test_fixtures::{layer, stack_summary};
    use crossterm::event::{KeyCode, KeyModifiers};

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
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .apply_action(&Action::ShowLayers(0), &shell, repo.as_path())
            .await;

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
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_stack_uses_top_layer_when_none_current() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_layer_uses_selected_layer_branch() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: Some(1),
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_returns_set_error_on_checkout_failure() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_with_message() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: Some("add new layer".to_string()),
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_without_message_flag() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_returns_set_error_on_failure() {
        let repo = std::env::current_dir().unwrap();
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
            .apply_action(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn add_layer_refuses_to_run_on_a_non_current_stack() {
        let repo = std::env::current_dir().unwrap();
        // No responses registered: the shell must not be invoked at all.
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let follow_ups = app
            .apply_action(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn load_layer_diff_diffs_bottom_layer_against_trunk() {
        let repo = std::env::current_dir().unwrap();
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

        app.apply_action(
            &Action::LoadLayerDiff {
                stack_index: 0,
                layer_index: 0,
                force: false,
            },
            &shell,
            repo.as_path(),
        )
        .await;

        let key = layer_diff_cache_key(&app.state.stacks[0], &app.state.stacks[0].layers[0]);
        assert_eq!(app.state.layer_diffs.get(&key).unwrap(), "+bottom");
    }

    #[tokio::test]
    async fn refresh_stacks_creates_started_action_without_blocking() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();

        let follow_ups = app
            .apply_action(&Action::RefreshStacks, &shell, repo.as_path())
            .await;

        assert!(matches!(
            follow_ups.as_slice(),
            [Action::StackRefreshStarted { request_id: 1 }]
        ));
        assert!(!app.state.refresh_in_flight);
    }

    #[tokio::test]
    async fn refresh_started_sets_loading_and_keeps_existing_stacks() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        app.apply_action(
            &Action::StackRefreshStarted { request_id: 9 },
            &shell,
            repo.as_path(),
        )
        .await;

        assert!(app.state.refresh_in_flight);
        assert_eq!(app.state.refresh_active_request_id, Some(9));
        assert_eq!(app.state.stacks.len(), 1);
        assert_eq!(app.state.last_successful_stacks.len(), 1);
    }

    #[tokio::test]
    async fn refresh_success_ignores_outdated_request_ids() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.refresh_active_request_id = Some(2);

        app.apply_action(
            &Action::StackRefreshSucceeded {
                request_id: 1,
                result: Ok(vec![stack_summary("stack-b", 1)]),
            },
            &shell,
            repo.as_path(),
        )
        .await;

        assert_eq!(app.state.stacks[0].label, "stack-a");
    }

    #[tokio::test]
    async fn refresh_failure_sets_dismissible_error() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.refresh_active_request_id = Some(4);
        app.state.refresh_in_flight = true;

        app.dispatch_actions_with_loader(
            vec![Action::StackRefreshSucceeded {
                request_id: 4,
                result: Err("network timeout".to_string()),
            }],
            &shell,
            repo.as_path(),
            None,
        )
        .await;

        assert!(!app.state.refresh_in_flight);
        assert!(app.state.error.is_some());
    }

    #[tokio::test]
    async fn tick_advances_spinner_while_refreshing() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.refresh_in_flight = true;

        app.apply_action(&Action::Tick, &shell, repo.as_path())
            .await;
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
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let actions = app.handle_key(key(KeyCode::Char('q')));
        assert!(matches!(actions.as_slice(), [Action::ShowConfirm(_)]));

        app.dispatch_actions_with_loader(actions, &shell, repo.as_path(), None)
            .await;

        let modal = app.state.confirm.as_ref().expect("modal should be shown");
        assert!(!modal.danger);
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn confirming_quit_modal_actually_quits() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_actions_with_loader(show, &shell, repo.as_path(), None)
            .await;
        assert!(app.state.confirm.is_some());
        assert!(!app.state.should_quit);

        let accept = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(accept.as_slice(), [Action::ConfirmAccept]));
        app.dispatch_actions_with_loader(accept, &shell, repo.as_path(), None)
            .await;

        assert!(app.state.should_quit);
        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn cancelling_quit_modal_leaves_app_running() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_actions_with_loader(show, &shell, repo.as_path(), None)
            .await;

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_actions_with_loader(cancel, &shell, repo.as_path(), None)
            .await;

        assert!(app.state.confirm.is_none());
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn danger_modal_requires_typed_phrase_before_enter_fires_action() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
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
        app.dispatch_actions_with_loader(premature, &shell, repo.as_path(), None)
            .await;
        assert!(app.state.confirm.is_some());

        // Typing the wrong phrase also keeps it open.
        for c in "no".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_actions_with_loader(typed, &shell, repo.as_path(), None)
                .await;
        }
        let premature = app.handle_key(key(KeyCode::Enter));
        app.dispatch_actions_with_loader(premature, &shell, repo.as_path(), None)
            .await;
        assert!(app.state.confirm.is_some());

        for _ in 0.."no".len() {
            let backspace = app.handle_key(key(KeyCode::Backspace));
            app.dispatch_actions_with_loader(backspace, &shell, repo.as_path(), None)
                .await;
        }

        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_actions_with_loader(typed, &shell, repo.as_path(), None)
                .await;
        }
        assert_eq!(app.state.confirm.as_ref().unwrap().typed_input(), "yes");

        let accept = app.handle_key(key(KeyCode::Enter));
        app.dispatch_actions_with_loader(accept, &shell, repo.as_path(), None)
            .await;

        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn danger_modal_esc_cancels_without_firing_action() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
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
            app.dispatch_actions_with_loader(typed, &shell, repo.as_path(), None)
                .await;
        }

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_actions_with_loader(cancel, &shell, repo.as_path(), None)
            .await;

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
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .apply_action(
                &Action::SubmitStack { stack_index: 0 },
                &shell,
                repo.as_path(),
            )
            .await;
        assert!(matches!(
            follow_ups.as_slice(),
            [Action::SubmitStarted { stack_index: 0 }]
        ));

        app.apply_action(&follow_ups[0], &shell, repo.as_path())
            .await;

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert_eq!(progress.total(), 2);
        assert_eq!(progress.completed_count(), 0);
        assert!(!progress.finished);
    }

    #[tokio::test]
    async fn submit_stack_action_refuses_to_start_a_second_run_while_one_is_in_progress() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.submit_progress =
            Some(SubmitProgress::new(0, vec!["stack-a-layer-0".to_string()]));

        let follow_ups = app
            .apply_action(
                &Action::SubmitStack { stack_index: 0 },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(follow_ups.is_empty());
        assert!(app.state.status.is_some());
    }

    #[tokio::test]
    async fn submit_layer_progress_action_updates_only_the_matching_layer() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(
            0,
            vec!["a".to_string(), "b".to_string()],
        ));

        app.apply_action(
            &Action::SubmitLayerProgress {
                stack_index: 0,
                layer_index: 1,
                status: LayerSubmitStatus::Failed("boom".to_string()),
            },
            &shell,
            repo.as_path(),
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
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        let follow_ups = app
            .apply_action(
                &Action::SubmitFinished { stack_index: 0 },
                &shell,
                repo.as_path(),
            )
            .await;

        assert!(app.state.submit_progress.as_ref().unwrap().finished);
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn toggle_actions_flip_submit_options() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        assert!(!app.state.submit_options.auto);
        assert!(!app.state.submit_options.open);

        app.apply_action(&Action::ToggleSubmitAuto, &shell, repo.as_path())
            .await;
        app.apply_action(&Action::ToggleSubmitOpen, &shell, repo.as_path())
            .await;

        assert!(app.state.submit_options.auto);
        assert!(app.state.submit_options.open);
    }

    #[tokio::test]
    async fn dismiss_submit_clears_progress() {
        let repo = std::env::current_dir().unwrap();
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        app.apply_action(&Action::DismissSubmit, &shell, repo.as_path())
            .await;

        assert!(app.state.submit_progress.is_none());
    }

    /// Reproduces the issue's acceptance criterion directly: layer 2 of 4
    /// fails to push while the other three succeed, and the failure must be
    /// visible on that layer alone rather than failing the whole submit.
    #[tokio::test]
    async fn run_submit_sequence_reports_a_partial_failure_per_layer() {
        let repo = std::env::current_dir().unwrap();
        let stack = stack_summary("stack-a", 4);
        let layers = stack.layers.clone();

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

        let mut results: Vec<(usize, LayerSubmitStatus)> = Vec::new();
        run_submit_sequence(
            &shell,
            repo.as_path(),
            "origin",
            &layers,
            SubmitOptions::default(),
            |layer_index, status| results.push((layer_index, status)),
        )
        .await;

        let terminal: Vec<_> = results
            .iter()
            .filter(|(_, status)| status.is_terminal())
            .cloned()
            .collect();

        assert_eq!(terminal.len(), 4);
        assert_eq!(
            terminal[0],
            (0, LayerSubmitStatus::PullRequestCreated { number: 1 })
        );
        assert!(matches!(&terminal[1], (1, LayerSubmitStatus::Failed(_))));
        assert_eq!(
            terminal[2],
            (2, LayerSubmitStatus::PullRequestCreated { number: 3 })
        );
        assert_eq!(
            terminal[3],
            (3, LayerSubmitStatus::PullRequestCreated { number: 4 })
        );
    }

    #[tokio::test]
    async fn run_submit_sequence_reports_every_layer_succeeding() {
        let repo = std::env::current_dir().unwrap();
        let stack = stack_summary("stack-a", 2);
        let layers = stack.layers.clone();

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

        let mut results: Vec<(usize, LayerSubmitStatus)> = Vec::new();
        run_submit_sequence(
            &shell,
            repo.as_path(),
            "origin",
            &layers,
            SubmitOptions::default(),
            |layer_index, status| results.push((layer_index, status)),
        )
        .await;

        let terminal: Vec<_> = results
            .into_iter()
            .filter(|(_, status)| status.is_terminal())
            .collect();
        assert_eq!(
            terminal,
            vec![
                (0, LayerSubmitStatus::PullRequestCreated { number: 1 }),
                (1, LayerSubmitStatus::PullRequestCreated { number: 2 }),
            ]
        );
    }
}
