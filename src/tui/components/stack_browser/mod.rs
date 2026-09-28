//! The unified stack browser: navigator, layer detail, changed files and
//! diff panels, with focus moving between them. Each panel renders from its
//! own submodule; this module owns the shared view state, key handling and
//! layout.

mod chrome;
mod diff;
mod layer_detail;
mod navigator;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{ListState, Paragraph};

use crate::stack::UnstackScope;
use crate::theme::ui::THEME;
use crate::tui::action::Action;
use crate::tui::component::Component;
use crate::tui::components::add_layer_prompt::{self, AddLayerPrompt, PromptOutcome};
use crate::tui::components::submit_progress;
use crate::tui::keymap::{KeyIntent, key_intent};
use crate::tui::state::{AppState, Screen, layer_diff_cache_key};
use crate::tui::widgets::panel_block;

use chrome::{render_footer, render_header};
use diff::parse_diff_files;
use layer_detail::render_layer_detail;
use navigator::render_navigator;

/// The browser view state the panels render from.
#[derive(Debug, Clone, Copy)]
struct PanelView {
    active_panel: ActivePanel,
    selected_diff_file: usize,
    diff_scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePanel {
    Stacks,
    Layers,
    Detail,
    Files,
    Diff,
}

pub struct StackBrowser {
    stack_list_state: ListState,
    list_state: ListState,
    active_stack_label: Option<String>,
    active_panel: ActivePanel,
    last_non_diff_panel: ActivePanel,
    selected_diff_file: usize,
    pending_g: bool,
    diff_scroll: u16,
    active_layer_key: Option<String>,
    add_layer_prompt: Option<AddLayerPrompt>,
}

impl StackBrowser {
    pub fn new() -> Self {
        Self {
            stack_list_state: ListState::default(),
            list_state: ListState::default(),
            active_stack_label: None,
            active_panel: ActivePanel::Stacks,
            last_non_diff_panel: ActivePanel::Detail,
            selected_diff_file: 0,
            pending_g: false,
            diff_scroll: 0,
            active_layer_key: None,
            add_layer_prompt: None,
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }
}

fn render(
    frame: &mut Frame,
    state: &AppState,
    stack_list_state: &mut ListState,
    list_state: &mut ListState,
    view: PanelView,
) {
    let [header_area, content_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    let selected_stack = selected_stack_index(state, stack_list_state.selected());
    let selected_layer = selected_stack
        .and_then(|index| state.stacks.get(index))
        .and_then(|stack| {
            list_state
                .selected()
                .and_then(|layer_index| stack.layers.get(layer_index))
        });
    render_header(frame, header_area, state, selected_stack, selected_layer);
    render_stack(
        frame,
        content_area,
        state,
        stack_list_state,
        list_state,
        selected_stack,
        view,
    );
    render_footer(frame, footer_area, state);

    if let Some(progress) = &state.submit_progress {
        submit_progress::render(frame, content_area, progress);
    }
}

fn render_stack(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    stack_list_state: &mut ListState,
    list_state: &mut ListState,
    selected_stack: Option<usize>,
    view: PanelView,
) {
    let active_panel = view.active_panel;
    let [stack_area, detail_area] = Layout::new(
        Direction::Horizontal,
        [Constraint::Percentage(32), Constraint::Percentage(68)],
    )
    .areas(area);

    render_navigator(
        frame,
        stack_area,
        state,
        stack_list_state.selected(),
        list_state.selected(),
        active_panel,
    );

    let Some(stack) = selected_stack.and_then(|index| state.stacks.get(index)) else {
        frame.render_widget(
            Paragraph::new("No stacks found in this repository.")
                .style(THEME.text.muted)
                .block(panel_block(
                    "navigator",
                    matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
                )),
            stack_area,
        );
        frame.render_widget(
            Paragraph::new("Select a stack to view its layers.")
                .style(THEME.text.muted)
                .block(panel_block("details", active_panel == ActivePanel::Detail)),
            detail_area,
        );
        return;
    };

    render_layer_detail(
        frame,
        detail_area,
        state,
        stack,
        list_state.selected(),
        view,
    );
}

impl Component for StackBrowser {
    fn draw(&mut self, frame: &mut Frame, state: &AppState) {
        render(
            frame,
            state,
            &mut self.stack_list_state,
            &mut self.list_state,
            PanelView {
                active_panel: self.active_panel,
                selected_diff_file: self.selected_diff_file,
                diff_scroll: self.diff_scroll,
            },
        );

        if let Some(prompt) = &self.add_layer_prompt {
            add_layer_prompt::render(frame, frame.area(), prompt);
        }
    }

    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action> {
        if self.add_layer_prompt.is_some() {
            return self.handle_add_layer_prompt_key(key);
        }

        if state.submit_progress.is_some() {
            return match key_intent(key) {
                Some(KeyIntent::Back) | Some(KeyIntent::DismissMessage) => {
                    vec![Action::DismissSubmit]
                }
                _ => Vec::new(),
            };
        }

        let selected_stack = selected_stack_index(state, self.stack_list_state.selected());
        let is_pending_g = self.pending_g;
        self.pending_g = false;

        if self.active_panel == ActivePanel::Diff {
            match key.code {
                KeyCode::Char('g') if is_pending_g => return vec![Action::ScrollDiffTop],
                KeyCode::Char('g') => {
                    self.pending_g = true;
                    return Vec::new();
                }
                _ => {}
            }
        }

        match key_intent(key) {
            Some(KeyIntent::Back) => vec![Action::Quit],
            Some(KeyIntent::FocusNext) => vec![Action::FocusNextPanel],
            Some(KeyIntent::FocusPrevious) => vec![Action::FocusPreviousPanel],
            Some(KeyIntent::MoveDown) => match self.active_panel {
                ActivePanel::Stacks => next_stack_action(state, selected_stack),
                ActivePanel::Layers => vec![Action::SelectNext],
                ActivePanel::Files => vec![Action::SelectNextDiffFile],
                ActivePanel::Diff => vec![Action::ScrollDiffLineDown],
                ActivePanel::Detail => Vec::new(),
            },
            Some(KeyIntent::MoveUp) => match self.active_panel {
                ActivePanel::Stacks => previous_stack_action(state, selected_stack),
                ActivePanel::Layers => vec![Action::SelectPrevious],
                ActivePanel::Files => vec![Action::SelectPreviousDiffFile],
                ActivePanel::Diff => vec![Action::ScrollDiffLineUp],
                ActivePanel::Detail => Vec::new(),
            },
            Some(KeyIntent::PageDown) => vec![Action::ScrollDiffDown],
            Some(KeyIntent::PageUp) => vec![Action::ScrollDiffUp],
            Some(KeyIntent::HalfPageDown) => vec![Action::ScrollDiffHalfPageDown],
            Some(KeyIntent::HalfPageUp) => vec![Action::ScrollDiffHalfPageUp],
            Some(KeyIntent::End) => vec![Action::ScrollDiffBottom],
            Some(KeyIntent::Refresh) => {
                if self.active_panel == ActivePanel::Stacks || selected_stack.is_none() {
                    vec![Action::RefreshStacks]
                } else {
                    self.list_state
                        .selected()
                        .map(|layer_index| {
                            let stack_index = selected_stack.unwrap_or_default();
                            vec![
                                Action::LoadLayerDetail {
                                    stack_index,
                                    layer_index,
                                    force: true,
                                },
                                Action::LoadLayerDiff {
                                    stack_index,
                                    layer_index,
                                    force: true,
                                },
                            ]
                        })
                        .unwrap_or_default()
                }
            }
            Some(KeyIntent::Checkout) => selected_stack
                .map(|stack_index| match self.active_panel {
                    ActivePanel::Stacks => vec![Action::CheckoutSelected {
                        stack_index,
                        layer_index: None,
                    }],
                    _ => self
                        .list_state
                        .selected()
                        .map(|layer_index| {
                            vec![Action::CheckoutSelected {
                                stack_index,
                                layer_index: Some(layer_index),
                            }]
                        })
                        .unwrap_or_default(),
                })
                .unwrap_or_default(),
            Some(KeyIntent::AddLayer) => {
                if let Some(stack_index) = selected_stack {
                    self.add_layer_prompt = Some(AddLayerPrompt::new(stack_index));
                }
                Vec::new()
            }
            Some(KeyIntent::Unstack) => selected_stack
                .map(|stack_index| {
                    vec![Action::UnstackSelected {
                        stack_index,
                        scope: UnstackScope::Local,
                    }]
                })
                .unwrap_or_default(),
            Some(KeyIntent::UnstackRemote) => selected_stack
                .map(|stack_index| {
                    vec![Action::UnstackSelected {
                        stack_index,
                        scope: UnstackScope::LocalAndRemote,
                    }]
                })
                .unwrap_or_default(),
            Some(KeyIntent::ToggleDiff) => vec![Action::ToggleDiffView],
            Some(KeyIntent::Submit) => selected_stack
                .map(|stack_index| vec![Action::SubmitStack { stack_index }])
                .unwrap_or_default(),
            Some(KeyIntent::ToggleSubmitAuto) => vec![Action::ToggleSubmitAuto],
            Some(KeyIntent::ToggleSubmitOpen) => vec![Action::ToggleSubmitOpen],
            Some(KeyIntent::DrillIn) => match self.active_panel {
                ActivePanel::Stacks => vec![Action::FocusNextPanel],
                ActivePanel::Layers => vec![Action::FocusNextPanel],
                ActivePanel::Detail => vec![Action::FocusNextPanel],
                ActivePanel::Files => vec![Action::ToggleDiffView],
                ActivePanel::Diff => Vec::new(),
            },
            Some(KeyIntent::OpenExternal) => selected_stack
                .and_then(|stack_index| {
                    self.list_state.selected().map(|layer_index| {
                        vec![Action::OpenPullRequest {
                            stack_index,
                            layer_index,
                        }]
                    })
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn update(&mut self, action: &Action, state: &mut AppState) {
        match action {
            Action::ShowLayers(stack_index) => {
                self.stack_list_state.select(Some(*stack_index));
                let Some(stack) = state.stacks.get(*stack_index) else {
                    return;
                };
                let count = stack.layers.len();
                let stack_label = Some(stack.label.clone());
                let preserve_selection =
                    self.active_stack_label.as_deref() == stack_label.as_deref();
                let next_selection =
                    clamped_selection(count, self.list_state.selected(), preserve_selection);
                self.list_state.select(next_selection);
                self.active_stack_label = stack_label;
                self.reset_diff_view_for_selection(state);
            }
            Action::SelectNext if matches!(state.screen, Screen::Layers(_)) => {
                select_next(&mut self.list_state, active_layer_count(state));
                self.reset_diff_view_for_selection(state);
            }
            Action::SelectPrevious if matches!(state.screen, Screen::Layers(_)) => {
                select_previous(&mut self.list_state, active_layer_count(state));
                self.reset_diff_view_for_selection(state);
            }
            Action::FocusNextPanel => {
                self.active_panel = next_panel(self.active_panel);
            }
            Action::FocusPreviousPanel => {
                self.active_panel = previous_panel(self.active_panel);
            }
            Action::SelectNextDiffFile => {
                self.select_next_diff_file(state);
            }
            Action::SelectPreviousDiffFile => {
                self.select_previous_diff_file(state);
            }
            Action::ScrollDiffDown => {
                self.scroll_diff_down();
            }
            Action::ScrollDiffUp => {
                self.scroll_diff_up();
            }
            Action::ScrollDiffHalfPageDown => {
                self.scroll_diff_half_page_down();
            }
            Action::ScrollDiffHalfPageUp => {
                self.scroll_diff_half_page_up();
            }
            Action::ScrollDiffLineDown => {
                self.scroll_diff_line_down();
            }
            Action::ScrollDiffLineUp => {
                self.scroll_diff_line_up();
            }
            Action::ScrollDiffTop => {
                self.scroll_diff_top();
            }
            Action::ScrollDiffBottom => {
                self.scroll_diff_bottom();
            }
            Action::ToggleDiffView => {
                if self.active_panel == ActivePanel::Diff {
                    self.active_panel = self.last_non_diff_panel;
                } else {
                    self.last_non_diff_panel = self.active_panel;
                    self.active_panel = ActivePanel::Diff;
                }
            }
            Action::StacksLoaded(selected_stack) => {
                let selected_stack = selected_stack
                    .or_else(|| selected_stack_index(state, self.stack_list_state.selected()));
                self.stack_list_state.select(selected_stack);
                let active_stack =
                    selected_stack.and_then(|stack_index| state.stacks.get(stack_index));
                let layer_count = active_stack.map(|stack| stack.layers.len()).unwrap_or(0);
                let active_label = active_stack.map(|stack| stack.label.clone());
                let preserve_selection =
                    self.active_stack_label.as_deref() == active_label.as_deref();
                let next_selection =
                    clamped_selection(layer_count, self.list_state.selected(), preserve_selection);
                self.list_state.select(next_selection);
                self.active_stack_label = active_label;
                self.reset_diff_view_for_selection(state);
            }
            _ => {}
        }
    }
}

impl StackBrowser {
    /// Routes a key event to the add-layer prompt while it's open.
    fn handle_add_layer_prompt_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(prompt) = self.add_layer_prompt.as_mut() else {
            return Vec::new();
        };

        match prompt.handle_key(key) {
            PromptOutcome::Editing => Vec::new(),
            PromptOutcome::Cancelled => {
                self.add_layer_prompt = None;
                Vec::new()
            }
            PromptOutcome::Submitted(action) => {
                self.add_layer_prompt = None;
                vec![action]
            }
        }
    }

    fn reset_diff_view_for_selection(&mut self, state: &AppState) {
        let next_key = active_layer_key(
            state,
            selected_stack_index(state, self.stack_list_state.selected()),
            self.list_state.selected(),
        );
        if self.active_layer_key != next_key {
            self.selected_diff_file = 0;
            self.diff_scroll = 0;
            self.active_layer_key = next_key;
        }
    }

    fn select_next_diff_file(&mut self, state: &AppState) {
        let count = active_diff_file_count(state, self.list_state.selected());
        if count == 0 {
            self.selected_diff_file = 0;
            return;
        }

        self.selected_diff_file = (self.selected_diff_file + 1) % count;
        self.diff_scroll = 0;
    }

    fn select_previous_diff_file(&mut self, state: &AppState) {
        let count = active_diff_file_count(state, self.list_state.selected());
        if count == 0 {
            self.selected_diff_file = 0;
            return;
        }

        self.selected_diff_file = if self.selected_diff_file == 0 {
            count - 1
        } else {
            self.selected_diff_file - 1
        };
        self.diff_scroll = 0;
    }

    fn scroll_diff_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(12);
    }

    fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(12);
    }

    fn scroll_diff_half_page_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(10);
    }

    fn scroll_diff_half_page_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(10);
    }

    fn scroll_diff_line_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(1);
    }

    fn scroll_diff_line_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    fn scroll_diff_top(&mut self) {
        self.diff_scroll = 0;
    }

    fn scroll_diff_bottom(&mut self) {
        self.diff_scroll = u16::MAX;
    }
}

fn next_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Layers,
        ActivePanel::Layers => ActivePanel::Detail,
        ActivePanel::Detail => ActivePanel::Files,
        ActivePanel::Files => ActivePanel::Diff,
        ActivePanel::Diff => ActivePanel::Stacks,
    }
}

fn previous_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Diff,
        ActivePanel::Layers => ActivePanel::Stacks,
        ActivePanel::Detail => ActivePanel::Layers,
        ActivePanel::Files => ActivePanel::Detail,
        ActivePanel::Diff => ActivePanel::Files,
    }
}

fn active_layer_key(
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
) -> Option<String> {
    let stack_index = selected_stack?;
    let stack = state.stacks.get(stack_index)?;
    let layer = stack.layers.get(selected_layer?)?;
    Some(layer_diff_cache_key(stack, layer))
}

fn active_layer_count(state: &AppState) -> usize {
    selected_stack_index(state, None)
        .and_then(|stack_index| state.stacks.get(stack_index))
        .map(|stack| stack.layers.len())
        .unwrap_or(0)
}

fn active_diff_file_count(state: &AppState, selected_layer: Option<usize>) -> usize {
    let Some(stack_index) = selected_stack_index(state, None) else {
        return 0;
    };
    let Some(stack) = state.stacks.get(stack_index) else {
        return 0;
    };
    let Some(layer) = selected_layer.and_then(|index| stack.layers.get(index)) else {
        return 0;
    };

    state
        .layer_diffs
        .get(&layer_diff_cache_key(stack, layer))
        .map(|diff| parse_diff_files(diff).len())
        .unwrap_or(0)
}

fn clamped_selection(
    count: usize,
    selected: Option<usize>,
    preserve_selection: bool,
) -> Option<usize> {
    if count == 0 {
        None
    } else if preserve_selection {
        match selected {
            Some(index) if index < count => Some(index),
            _ => Some(0),
        }
    } else {
        Some(0)
    }
}

fn selected_stack_index(state: &AppState, fallback: Option<usize>) -> Option<usize> {
    match state.screen {
        Screen::Layers(index) if index < state.stacks.len() => Some(index),
        _ => fallback
            .filter(|index| *index < state.stacks.len())
            .or_else(|| state.stacks.iter().position(|stack| stack.is_current))
            .or(if state.stacks.is_empty() {
                None
            } else {
                Some(0)
            }),
    }
}

fn next_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let next = match selected {
        Some(index) if index + 1 < state.stacks.len() => index + 1,
        _ => 0,
    };

    vec![Action::ShowLayers(next)]
}

fn previous_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let previous = match selected {
        Some(0) | None => state.stacks.len() - 1,
        Some(index) => index - 1,
    };

    vec![Action::ShowLayers(previous)]
}

fn select_next(state: &mut ListState, count: usize) {
    if count == 0 {
        state.select(None);
        return;
    }

    let next = match state.selected() {
        Some(index) if index + 1 < count => index + 1,
        _ => 0,
    };

    state.select(Some(next));
}

fn select_previous(state: &mut ListState, count: usize) {
    if count == 0 {
        state.select(None);
        return;
    }

    let previous = match state.selected() {
        Some(0) | None => count - 1,
        Some(index) => index - 1,
    };

    state.select(Some(previous))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{app_state, stack_summary};
    use crate::tui::components::add_layer_prompt::AddLayerField;
    use crate::tui::state::submit_progress::SubmitProgress;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn sample_diff_with_paths() -> String {
        [
            "diff --git a/src/a.rs b/src/a.rs",
            "index 123..456 100644",
            "--- a/src/a.rs",
            "+++ b/src/a.rs",
            "@@ -1 +1 @@",
            "-old",
            "+new",
            "diff --git a/src/nested/b.rs b/src/nested/b.rs",
            "index 789..abc 100644",
            "--- a/src/nested/b.rs",
            "+++ b/src/nested/b.rs",
            "@@ -3 +3 @@",
            "+more",
        ]
        .join("\n")
    }

    #[test]
    fn show_layers_preserves_selection_for_same_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::ShowLayers(0), &mut state);

        assert_eq!(component.list_state.selected(), Some(1));
    }

    #[test]
    fn show_layers_resets_selection_when_switching_stacks() {
        let mut component = StackBrowser::new();
        let mut state = app_state(
            vec![stack_summary("a", 3), stack_summary("b", 3)],
            Screen::Layers(0),
        );

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::ShowLayers(1), &mut state);

        assert_eq!(component.stack_list_state.selected(), Some(1));
        assert_eq!(component.list_state.selected(), Some(0));
    }

    #[test]
    fn stacks_loaded_preserves_selection_for_same_stack_label() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        state.stacks = vec![stack_summary("x", 1), stack_summary("a", 3)];
        state.screen = Screen::Layers(1);
        component.update(&Action::StacksLoaded(Some(1)), &mut state);

        assert_eq!(component.list_state.selected(), Some(1));
    }

    #[test]
    fn stacks_loaded_resets_selection_for_different_stack_label() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        state.stacks = vec![stack_summary("b", 3)];
        state.screen = Screen::Layers(0);
        component.update(&Action::StacksLoaded(Some(0)), &mut state);

        assert_eq!(component.list_state.selected(), Some(0));
    }

    #[test]
    fn handle_key_maps_navigation_actions() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let quit = component.handle_key(key(KeyCode::Char('q')), &state);
        let next = component.handle_key(key(KeyCode::Down), &state);
        let previous = component.handle_key(key(KeyCode::Up), &state);
        let refresh = component.handle_key(key(KeyCode::Char('r')), &state);

        assert!(matches!(quit.as_slice(), [Action::Quit]));
        assert!(matches!(next.as_slice(), [Action::SelectNext]));
        assert!(matches!(previous.as_slice(), [Action::SelectPrevious]));
        assert!(matches!(
            refresh.as_slice(),
            [
                Action::LoadLayerDetail {
                    stack_index: 0,
                    layer_index: 0,
                    force: true,
                },
                Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: true,
                }
            ]
        ));
    }

    #[test]
    fn handle_key_dispatches_open_pr_for_selected_layer() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let actions = component.handle_key(key(KeyCode::Char('o')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::OpenPullRequest {
                stack_index: 0,
                layer_index: 1
            }]
        ));
    }

    #[test]
    fn handle_key_dispatches_checkout_for_selected_layer() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let actions = component.handle_key(key(KeyCode::Char('c')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::CheckoutSelected {
                stack_index: 0,
                layer_index: Some(1)
            }]
        ));
    }

    #[test]
    fn handle_key_dispatches_checkout_for_selected_stack_from_stack_panel() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('c')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::CheckoutSelected {
                stack_index: 0,
                layer_index: None
            }]
        ));
    }

    #[test]
    fn a_opens_add_layer_prompt_on_branch_field() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('a')), &state);

        assert!(actions.is_empty());
        let prompt = component.add_layer_prompt.as_ref().expect("prompt open");
        assert_eq!(prompt.stack_index, 0);
        assert_eq!(prompt.field, AddLayerField::Branch);
    }

    #[test]
    fn add_layer_prompt_types_into_branch_then_advances_on_enter() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);

        // Global shortcut letters are captured as text, not intents, while
        // the prompt is open.
        component.handle_key(key(KeyCode::Char('c')), &state);
        component.handle_key(key(KeyCode::Char('q')), &state);
        assert_eq!(component.add_layer_prompt.as_ref().unwrap().branch, "cq");

        // Enter with an empty branch is a no-op.
        let mut empty_component = StackBrowser::new();
        empty_component.update(&Action::ShowLayers(0), &mut state);
        empty_component.handle_key(key(KeyCode::Char('a')), &state);
        empty_component.handle_key(key(KeyCode::Enter), &state);
        assert_eq!(
            empty_component.add_layer_prompt.as_ref().unwrap().field,
            AddLayerField::Branch
        );

        let advance = component.handle_key(key(KeyCode::Enter), &state);
        assert!(advance.is_empty());
        assert_eq!(
            component.add_layer_prompt.as_ref().unwrap().field,
            AddLayerField::Message
        );
    }

    #[test]
    fn add_layer_prompt_backspace_removes_last_character() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        component.handle_key(key(KeyCode::Char('x')), &state);
        component.handle_key(key(KeyCode::Char('y')), &state);

        component.handle_key(key(KeyCode::Backspace), &state);

        assert_eq!(component.add_layer_prompt.as_ref().unwrap().branch, "x");
    }

    #[test]
    fn escape_cancels_add_layer_prompt_without_dispatching_actions() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        component.handle_key(key(KeyCode::Char('x')), &state);

        let actions = component.handle_key(key(KeyCode::Esc), &state);

        assert!(actions.is_empty());
        assert!(component.add_layer_prompt.is_none());
    }

    #[test]
    fn enter_on_message_field_submits_add_layer_with_message_and_closes_prompt() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        for c in "feature/new".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }
        component.handle_key(key(KeyCode::Enter), &state);
        for c in "add a layer".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }

        let actions = component.handle_key(key(KeyCode::Enter), &state);

        assert!(component.add_layer_prompt.is_none());
        assert!(matches!(
            actions.as_slice(),
            [Action::AddLayer {
                stack_index: 0,
                branch,
                message: Some(message),
            }] if branch == "feature/new" && message == "add a layer"
        ));
    }

    #[test]
    fn enter_on_blank_message_field_submits_add_layer_without_message() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        for c in "feature/new".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }
        component.handle_key(key(KeyCode::Enter), &state);

        let actions = component.handle_key(key(KeyCode::Enter), &state);

        assert!(component.add_layer_prompt.is_none());
        assert!(matches!(
            actions.as_slice(),
            [Action::AddLayer {
                stack_index: 0,
                branch,
                message: None,
            }] if branch == "feature/new"
        ));
    }

    #[test]
    fn d_toggles_diff_and_jk_scroll_it() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let toggle = component.handle_key(key(KeyCode::Char('d')), &state);
        assert!(matches!(toggle.as_slice(), [Action::ToggleDiffView]));
        component.update(&toggle[0], &mut state);

        let diff_down = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(diff_down.as_slice(), [Action::ScrollDiffLineDown]));
        component.update(&diff_down[0], &mut state);
        assert_eq!(component.diff_scroll, 1);
    }

    #[test]
    fn stack_panel_navigation_switches_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(
            vec![stack_summary("a", 1), stack_summary("b", 1)],
            Screen::Layers(0),
        );
        component.update(&Action::ShowLayers(0), &mut state);

        let next = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(next.as_slice(), [Action::ShowLayers(1)]));

        state.screen = Screen::Layers(1);
        component.update(&next[0], &mut state);
        assert_eq!(component.stack_list_state.selected(), Some(1));

        let refresh = component.handle_key(key(KeyCode::Char('r')), &state);
        assert!(matches!(refresh.as_slice(), [Action::RefreshStacks]));
    }

    #[test]
    fn enter_advances_focus_and_space_opens_diff_from_files() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let stack = state.stacks[0].clone();
        let layer = stack.layers[0].clone();
        state.layer_diffs.store_result(
            layer_diff_cache_key(&stack, &layer),
            Ok(sample_diff_with_paths()),
        );
        component.update(&Action::ShowLayers(0), &mut state);

        let stack_open = component.handle_key(key(KeyCode::Enter), &state);
        assert!(matches!(stack_open.as_slice(), [Action::FocusNextPanel]));
        component.update(&stack_open[0], &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        assert_eq!(component.active_panel, ActivePanel::Files);

        let open_diff = component.handle_key(key(KeyCode::Char(' ')), &state);
        assert!(matches!(open_diff.as_slice(), [Action::ToggleDiffView]));
    }

    #[test]
    fn files_panel_navigation_changes_selected_file() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let stack = state.stacks[0].clone();
        let layer = stack.layers[0].clone();
        state.layer_diffs.store_result(
            layer_diff_cache_key(&stack, &layer),
            Ok(sample_diff_with_paths()),
        );
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let next = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(next.as_slice(), [Action::SelectNextDiffFile]));
        component.update(&next[0], &mut state);
        assert_eq!(component.selected_diff_file, 1);
    }

    #[test]
    fn diff_panel_supports_vim_navigation() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::ToggleDiffView, &mut state);

        let first_g = component.handle_key(key(KeyCode::Char('g')), &state);
        assert!(first_g.is_empty());
        let second_g = component.handle_key(key(KeyCode::Char('g')), &state);
        assert!(matches!(second_g.as_slice(), [Action::ScrollDiffTop]));
        let end = component.handle_key(key(KeyCode::Char('G')), &state);
        assert!(matches!(end.as_slice(), [Action::ScrollDiffBottom]));
        let half_down = component.handle_key(ctrl_key(KeyCode::Char('d')), &state);
        assert!(matches!(
            half_down.as_slice(),
            [Action::ScrollDiffHalfPageDown]
        ));
        let half_up = component.handle_key(ctrl_key(KeyCode::Char('u')), &state);
        assert!(matches!(half_up.as_slice(), [Action::ScrollDiffHalfPageUp]));
    }

    #[test]
    fn handle_key_dispatches_submit_for_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('s')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::SubmitStack { stack_index: 0 }]
        ));
    }

    #[test]
    fn handle_key_dispatches_unstack_scopes_for_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let local = component.handle_key(key(KeyCode::Char('D')), &state);
        assert!(matches!(
            local.as_slice(),
            [Action::UnstackSelected {
                stack_index: 0,
                scope: UnstackScope::Local
            }]
        ));
        let remote = component.handle_key(key(KeyCode::Char('U')), &state);
        assert!(matches!(
            remote.as_slice(),
            [Action::UnstackSelected {
                stack_index: 0,
                scope: UnstackScope::LocalAndRemote
            }]
        ));
    }

    #[test]
    fn handle_key_toggles_submit_auto_and_open() {
        let mut component = StackBrowser::new();
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));

        let auto = component.handle_key(key(KeyCode::Char('t')), &state);
        assert!(matches!(auto.as_slice(), [Action::ToggleSubmitAuto]));

        let open = component.handle_key(key(KeyCode::Char('p')), &state);
        assert!(matches!(open.as_slice(), [Action::ToggleSubmitOpen]));
    }

    #[test]
    fn handle_key_while_submitting_only_allows_dismissal() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        state.submit_progress = Some(SubmitProgress::new(
            0,
            vec!["a-layer-0".to_string(), "a-layer-1".to_string()],
        ));

        let navigate = component.handle_key(key(KeyCode::Down), &state);
        assert!(navigate.is_empty());

        let dismiss = component.handle_key(key(KeyCode::Esc), &state);
        assert!(matches!(dismiss.as_slice(), [Action::DismissSubmit]));

        let dismiss_q = component.handle_key(key(KeyCode::Char('q')), &state);
        assert!(matches!(dismiss_q.as_slice(), [Action::DismissSubmit]));
    }
}
