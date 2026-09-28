//! The unified stack browser: navigator, layer detail, changed files and
//! diff panels, with focus moving between them. Each panel renders from its
//! own submodule; this module owns the shared view state, key handling and
//! layout.

mod chrome;
mod diff;
mod keys;
mod layer_detail;
mod navigator;
mod selection;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{ListState, Paragraph};

use crate::theme::ui::THEME;
use crate::tui::action::Action;
use crate::tui::component::Component;
use crate::tui::components::add_layer_prompt::{self, AddLayerPrompt};
use crate::tui::components::submit_progress;
use crate::tui::state::AppState;
use crate::tui::widgets::panel_block;

use chrome::{render_footer, render_header};
use layer_detail::render_layer_detail;
use navigator::render_navigator;
use selection::selected_stack_index;

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
        self.handle_key_event(key, state)
    }

    fn update(&mut self, action: &Action, state: &mut AppState) {
        self.apply_update(action, state);
    }
}

#[cfg(test)]
mod test_support {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    pub(super) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    pub(super) fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    pub(super) fn sample_diff_with_paths() -> String {
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
}
