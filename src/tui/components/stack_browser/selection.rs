//! Selection, focus and scroll state: how the browser's view state reacts to
//! applied [`Action`]s.

use ratatui::widgets::ListState;

use crate::tui::action::Action;
use crate::tui::state::{AppState, Screen, layer_diff_cache_key};

use super::diff::parse_diff_files;
use super::{ActivePanel, StackBrowser};

impl StackBrowser {
    pub(super) fn apply_update(&mut self, action: &Action, state: &mut AppState) {
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

pub(super) fn next_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Layers,
        ActivePanel::Layers => ActivePanel::Detail,
        ActivePanel::Detail => ActivePanel::Files,
        ActivePanel::Files => ActivePanel::Diff,
        ActivePanel::Diff => ActivePanel::Stacks,
    }
}

pub(super) fn previous_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Diff,
        ActivePanel::Layers => ActivePanel::Stacks,
        ActivePanel::Detail => ActivePanel::Layers,
        ActivePanel::Files => ActivePanel::Detail,
        ActivePanel::Diff => ActivePanel::Files,
    }
}

pub(super) fn active_layer_key(
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
) -> Option<String> {
    let stack_index = selected_stack?;
    let stack = state.stacks.get(stack_index)?;
    let layer = stack.layers.get(selected_layer?)?;
    Some(layer_diff_cache_key(stack, layer))
}

pub(super) fn active_layer_count(state: &AppState) -> usize {
    selected_stack_index(state, None)
        .and_then(|stack_index| state.stacks.get(stack_index))
        .map(|stack| stack.layers.len())
        .unwrap_or(0)
}

pub(super) fn active_diff_file_count(state: &AppState, selected_layer: Option<usize>) -> usize {
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

pub(super) fn clamped_selection(
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

pub(super) fn selected_stack_index(state: &AppState, fallback: Option<usize>) -> Option<usize> {
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

pub(super) fn next_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let next = match selected {
        Some(index) if index + 1 < state.stacks.len() => index + 1,
        _ => 0,
    };

    vec![Action::ShowLayers(next)]
}

pub(super) fn previous_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let previous = match selected {
        Some(0) | None => state.stacks.len() - 1,
        Some(index) => index - 1,
    };

    vec![Action::ShowLayers(previous)]
}

pub(super) fn select_next(state: &mut ListState, count: usize) {
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

pub(super) fn select_previous(state: &mut ListState, count: usize) {
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
    use crate::tui::component::Component;
    use crate::tui::state::Screen;

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
}
