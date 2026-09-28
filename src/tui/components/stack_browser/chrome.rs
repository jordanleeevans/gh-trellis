//! The header (selected stack/layer summary) and footer (errors, refresh
//! progress, key hints) around the browser.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::stack::Layer;
use crate::theme::ui::THEME;
use crate::tui::state::AppState;
use crate::tui::widgets::{glyphs, spinner_frame};

use super::navigator::stack_name;

pub(super) fn render_header(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<&Layer>,
) {
    let header = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Trellis ",
                Style::default()
                    .fg(THEME.colors.text_inverse)
                    .bg(THEME.colors.primary),
            ),
            Span::styled(" stacks", THEME.text.heading.fg(THEME.colors.secondary)),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(THEME.primary_border());

    let content = if let Some(layer) = selected_layer {
        header_summary_line(area, layer.branch.clone(), layer_status_text(layer))
    } else {
        Line::from(Span::raw(
            selected_stack
                .and_then(|index| state.stacks.get(index))
                .map(|stack| format!("{} (trunk: {})", stack_name(stack), stack.trunk))
                .unwrap_or_else(|| "Browse locally tracked stacks and layer status".to_string()),
        ))
    };

    frame.render_widget(
        Paragraph::new(content)
            .style(
                Style::default()
                    .fg(THEME.colors.secondary)
                    .add_modifier(Modifier::BOLD),
            )
            .block(header),
        area,
    );
}

pub(super) fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let content = footer_line(state);

    frame.render_widget(
        Paragraph::new(content).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(THEME.colors.text_muted)),
        ),
        area,
    );
}

pub(super) fn footer_line(state: &AppState) -> Line<'static> {
    if let Some(error) = &state.error {
        Line::from(vec![
            Span::styled("error: ", THEME.text.key.fg(THEME.colors.danger)),
            Span::raw(error.clone()),
            Span::raw("  "),
            Span::styled("x", THEME.text.key.fg(THEME.colors.warning)),
            Span::raw(" dismiss"),
        ])
    } else if state.sync_in_flight {
        Line::from(vec![
            Span::styled(
                format!("{} ", spinner_frame(state.refresh_spinner_frame)),
                THEME.text.key.fg(THEME.colors.secondary),
            ),
            Span::raw("Syncing stack (fetch, rebase, push)"),
        ])
    } else if let Some(notice) = &state.sync_notice {
        Line::from(vec![
            Span::styled("sync: ", THEME.text.key.fg(THEME.colors.success)),
            Span::raw(notice.clone()),
            Span::raw("  "),
            Span::styled("x", THEME.text.key.fg(THEME.colors.warning)),
            Span::raw(" dismiss"),
        ])
    } else if state.refresh_in_flight {
        Line::from(vec![
            Span::styled(
                format!("{} ", spinner_frame(state.refresh_spinner_frame)),
                THEME.text.key.fg(THEME.colors.secondary),
            ),
            Span::raw("Refreshing stacks in background"),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k", THEME.text.key),
            Span::raw(" navigate  "),
            Span::styled("space/enter", THEME.text.key),
            Span::raw(" open  "),
            Span::styled("tab/shift-tab", THEME.text.key),
            Span::raw(" focus  "),
            Span::styled("o", THEME.text.key),
            Span::raw(" PR  "),
            Span::styled("d", THEME.text.key),
            Span::raw(" diff  "),
            Span::styled("c", THEME.text.key),
            Span::raw(" checkout  "),
            Span::styled("a", THEME.text.key),
            Span::raw(" add layer  "),
            Span::styled("gg/G ^u/^d", THEME.text.key),
            Span::raw(" diff jump  "),
            Span::styled("s", THEME.text.key),
            Span::raw(" submit  "),
            Span::styled("t", THEME.text.key),
            Span::raw(format!(
                " auto:{}  ",
                toggle_label(state.submit_options.auto)
            )),
            Span::styled("p", THEME.text.key),
            Span::raw(format!(
                " open:{}  ",
                toggle_label(state.submit_options.open)
            )),
            Span::styled("S", THEME.text.key),
            Span::raw(" sync  "),
            Span::styled("P", THEME.text.key),
            Span::raw(format!(
                " prune:{}  ",
                toggle_label(state.sync_options.prune)
            )),
            Span::styled("r", THEME.text.key),
            Span::raw(" refresh  "),
            Span::styled("q", THEME.text.key.fg(THEME.colors.danger)),
            Span::raw(" quit"),
        ])
    }
}

pub(super) fn toggle_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

pub(super) fn header_summary_line(
    area: Rect,
    left: String,
    right: (String, Style),
) -> Line<'static> {
    let width = area.width.saturating_sub(4) as usize;
    let right_len = right.0.chars().count();
    let left_len = left.chars().count();
    let spacer_len = width.saturating_sub(left_len + right_len).max(1);
    Line::from(vec![
        Span::raw(left),
        Span::raw(" ".repeat(spacer_len)),
        Span::styled(right.0, right.1),
    ])
}

pub(super) fn layer_status_text(layer: &Layer) -> (String, Style) {
    if layer.needs_rebase {
        (
            format!("{} needs rebase", glyphs().warning),
            Style::default()
                .fg(THEME.colors.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            format!("{} up to date", glyphs().up),
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{app_state, stack_summary};
    use crate::tui::state::Screen;

    #[test]
    fn footer_shows_refresh_indicator_when_loading() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.refresh_in_flight = true;
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("Refreshing stacks"));
    }

    #[test]
    fn footer_shows_dismissible_error() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.error = Some("auth required".to_string());
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("error:"));
        assert!(text.contains("dismiss"));
    }

    #[test]
    fn footer_shows_submit_toggle_state() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.submit_options.auto = true;
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("submit"));
        assert!(text.contains("auto:on"));
        assert!(text.contains("open:off"));
    }

    #[test]
    fn footer_shows_sync_hint_and_prune_state() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let text = |state: &AppState| {
            footer_line(state)
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        assert!(text(&state).contains("sync"));
        assert!(text(&state).contains("prune:off"));

        state.sync_options.prune = true;
        assert!(text(&state).contains("prune:on"));

        state.sync_in_flight = true;
        assert!(text(&state).contains("Syncing stack"));
    }
}
