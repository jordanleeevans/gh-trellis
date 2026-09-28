//! The left-hand navigator: every stack, with the selected one expanded
//! into its layers.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::stack::{Layer, StackSummary};
use crate::theme::ui::THEME;
use crate::tui::state::AppState;
use crate::tui::widgets::{glyphs, panel_block};

use super::ActivePanel;

pub(super) fn render_navigator(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
    active_panel: ActivePanel,
) {
    if state.stacks.is_empty() {
        frame.render_widget(
            Paragraph::new("No stacks found in this repository.")
                .style(THEME.text.muted)
                .block(panel_block(
                    "navigator",
                    matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
                )),
            area,
        );
        return;
    }

    let lines = navigator_lines(state, selected_stack, selected_layer, active_panel);
    frame.render_widget(
        Paragraph::new(lines).block(panel_block(
            "navigator",
            matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
        )),
        area,
    );
}

pub(super) fn navigator_lines(
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
    active_panel: ActivePanel,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for (stack_index, stack) in state.stacks.iter().enumerate() {
        let expanded = Some(stack_index) == selected_stack;
        let stack_style = if expanded && active_panel == ActivePanel::Stacks {
            THEME.text.selected
        } else if stack.is_current {
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let symbol = if expanded { "▼" } else { "▶" };
        lines.push(Line::from(Span::styled(
            format!("{symbol} {}", stack_name(stack)),
            stack_style,
        )));

        if expanded {
            lines.push(Line::from(Span::styled(
                format!("  trunk: {}", stack.trunk),
                THEME.text.muted,
            )));
            lines.push(Line::from(""));

            for (layer_index, layer) in stack.layers.iter().enumerate() {
                let branch_marker = if layer_index + 1 == stack.layers.len() {
                    "└─"
                } else {
                    "├─"
                };
                let style =
                    if selected_layer == Some(layer_index) && active_panel == ActivePanel::Layers {
                        THEME.text.selected
                    } else if layer.is_current {
                        Style::default()
                            .fg(THEME.colors.success)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {branch_marker} {:<14} {}",
                        layer_title(layer),
                        layer_badge(layer)
                    ),
                    style,
                )));
            }

            lines.push(Line::from(""));
        }
    }

    lines
}

pub(super) fn stack_name(stack: &StackSummary) -> String {
    let Some(first_prefix) = stack
        .layers
        .first()
        .and_then(|layer| layer.branch.rsplit_once('/').map(|(prefix, _)| prefix))
    else {
        return stack.label.clone();
    };

    if stack
        .layers
        .iter()
        .all(|layer| layer.branch.rsplit_once('/').map(|(prefix, _)| prefix) == Some(first_prefix))
    {
        first_prefix.to_string()
    } else {
        stack.label.clone()
    }
}

pub(super) fn layer_title(layer: &Layer) -> String {
    layer
        .branch
        .rsplit('/')
        .next()
        .unwrap_or(layer.branch.as_str())
        .to_string()
}

pub(super) fn layer_badge(layer: &Layer) -> String {
    match &layer.pull_request {
        Some(pr) if layer.is_merged => format!("#{} {}", pr.number, glyphs().check),
        Some(pr) if pr.is_draft == Some(true) => format!("#{} {}", pr.number, glyphs().pending),
        Some(pr) if layer.needs_rebase => format!("#{} {}", pr.number, glyphs().warning),
        Some(pr) => format!("#{} {}", pr.number, glyphs().current),
        None => "unsubmitted".to_string(),
    }
}
