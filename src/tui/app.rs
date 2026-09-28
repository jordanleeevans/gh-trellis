use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use tokio::sync::mpsc;

use crate::git;
use crate::shell::{ProcessShell, Shell, ShellError};
use crate::stack::hydrate_layer_detail;
use crate::stack::{Layer, LayerDetail, StackSummary, list_stacks};

use super::keymap::{KeyIntent, key_intent};
use super::layer_resource::LayerResourceCache;
use super::stack_layers;

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
    }

    fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let mut actions = self.stack_layers.handle_key(key, &self.state);

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
            _ => false,
        }
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
}
