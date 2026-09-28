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

use crate::tui::keymap::{self, KeyIntent, Keymap};

use super::ActivePanel;
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

pub(super) fn render_footer(frame: &mut Frame, area: Rect, state: &AppState, panel: ActivePanel) {
    let content = footer_line(state, keymap::current(), panel, area.width);

    frame.render_widget(
        Paragraph::new(content).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(THEME.colors.text_muted)),
        ),
        area,
    );
}

type Hint = (&'static [KeyIntent], &'static str);

/// Key hints for the focused panel, most useful first. The footer shows as
/// many as fit; everything else is in the `?` help overlay.
fn panel_hints(panel: ActivePanel) -> &'static [Hint] {
    use KeyIntent::*;
    match panel {
        ActivePanel::Stacks => &[
            (&[MoveDown, MoveUp], "stacks"),
            (&[FocusNext], "panels"),
            (&[Checkout], "checkout"),
            (&[Submit], "submit"),
            (&[Sync], "sync"),
            (&[AddLayer], "add layer"),
            (&[Refresh], "refresh"),
        ],
        ActivePanel::Layers => &[
            (&[MoveDown, MoveUp], "layers"),
            (&[FocusNext], "panels"),
            (&[Checkout], "checkout"),
            (&[OpenExternal], "open PR"),
            (&[ToggleDiff], "diff"),
            (&[AddLayer], "add layer"),
        ],
        ActivePanel::Detail => &[
            (&[FocusNext], "panels"),
            (&[OpenExternal], "open PR"),
            (&[ToggleDiff], "diff"),
            (&[Refresh], "reload"),
        ],
        ActivePanel::Files => &[
            (&[MoveDown, MoveUp], "files"),
            (&[DrillIn], "view diff"),
            (&[FocusNext], "panels"),
            (&[OpenExternal], "open PR"),
        ],
        ActivePanel::Diff => &[
            (&[MoveDown, MoveUp], "scroll"),
            (&[HalfPageDown, HalfPageUp], "half page"),
            (&[End], "bottom"),
            (&[ToggleDiff], "close diff"),
        ],
    }
}

/// Always shown, at the end of the footer.
const TRAILING_HINTS: &[Hint] = &[(&[KeyIntent::Help], "help"), (&[KeyIntent::Back], "quit")];

/// `j/k`-style label for a hint's intents, skipping unbound ones.
fn hint_keys(keymap: &Keymap, intents: &[KeyIntent]) -> Option<String> {
    let labels: Vec<String> = intents
        .iter()
        .filter_map(|intent| keymap.short_label(*intent))
        .collect();
    (!labels.is_empty()).then(|| labels.join("/"))
}

fn hint_spans(keys: String, description: &str) -> [Span<'static>; 2] {
    [
        Span::styled(keys, THEME.text.key),
        Span::raw(format!(" {description}  ")),
    ]
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

pub(super) fn footer_line(
    state: &AppState,
    keymap: &Keymap,
    panel: ActivePanel,
    width: u16,
) -> Line<'static> {
    let dismiss = || {
        [
            Span::raw("  "),
            Span::styled(
                keymap
                    .short_label(KeyIntent::DismissMessage)
                    .unwrap_or_default(),
                THEME.text.key.fg(THEME.colors.warning),
            ),
            Span::raw(" dismiss"),
        ]
    };

    if let Some(error) = &state.error {
        let mut spans = vec![
            Span::styled("error: ", THEME.text.key.fg(THEME.colors.danger)),
            Span::raw(error.clone()),
        ];
        spans.extend(dismiss());
        return Line::from(spans);
    }
    if state.sync_in_flight || state.refresh_in_flight {
        let message = if state.sync_in_flight {
            "Syncing stack (fetch, rebase, push)"
        } else {
            "Refreshing stacks in background"
        };
        return Line::from(vec![
            Span::styled(
                format!("{} ", spinner_frame(state.refresh_spinner_frame)),
                THEME.text.key.fg(THEME.colors.secondary),
            ),
            Span::raw(message),
        ]);
    }
    if let Some(notice) = &state.sync_notice {
        let mut spans = vec![
            Span::styled("sync: ", THEME.text.key.fg(THEME.colors.success)),
            Span::raw(notice.clone()),
        ];
        spans.extend(dismiss());
        return Line::from(spans);
    }

    let trailing: Vec<Span> = TRAILING_HINTS
        .iter()
        .filter_map(|(intents, description)| {
            hint_keys(keymap, intents).map(|keys| hint_spans(keys, description))
        })
        .flatten()
        .collect();
    let budget = usize::from(width).saturating_sub(spans_width(&trailing));

    let mut spans = Vec::new();
    for (intents, description) in panel_hints(panel) {
        let Some(keys) = hint_keys(keymap, intents) else {
            continue;
        };
        let hint = hint_spans(keys, description);
        if spans_width(&spans) + spans_width(&hint) > budget {
            break;
        }
        spans.extend(hint);
    }
    spans.extend(trailing);
    Line::from(spans)
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

    fn text(line: Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn footer(state: &AppState, panel: ActivePanel, width: u16) -> String {
        text(footer_line(state, &Keymap::default_keymap(), panel, width))
    }

    const PANELS: [ActivePanel; 5] = [
        ActivePanel::Stacks,
        ActivePanel::Layers,
        ActivePanel::Detail,
        ActivePanel::Files,
        ActivePanel::Diff,
    ];

    #[test]
    fn footer_fits_the_width_and_always_offers_help_and_quit() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        for width in [40, 80, 120, 200] {
            for panel in PANELS {
                let line = footer(&state, panel, width);
                assert!(
                    line.trim_end().chars().count() <= usize::from(width),
                    "{panel:?} at {width}: {line:?}"
                );
                assert!(line.contains("? help"), "{panel:?} at {width}: {line:?}");
                assert!(line.contains("q quit"), "{panel:?} at {width}: {line:?}");
            }
        }
    }

    #[test]
    fn footer_hints_follow_the_focused_panel() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let stacks = footer(&state, ActivePanel::Stacks, 200);
        assert!(stacks.contains("j/k stacks"));
        assert!(stacks.contains("s submit"));
        assert!(stacks.contains("S sync"));

        let diff = footer(&state, ActivePanel::Diff, 200);
        assert!(diff.contains("j/k scroll"));
        assert!(diff.contains("ctrl+d/ctrl+u half page"));
        assert!(!diff.contains("submit"));
    }

    #[test]
    fn footer_drops_lower_priority_hints_first_when_narrow() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let narrow = footer(&state, ActivePanel::Stacks, 40);
        assert!(narrow.contains("j/k stacks"));
        assert!(!narrow.contains("refresh"));
    }

    #[test]
    fn footer_labels_follow_rebound_keys() {
        use crate::config::keymap::KeyBinding;
        use crossterm::event::{KeyCode, KeyModifiers};
        use std::collections::BTreeMap;

        let mut overrides = BTreeMap::new();
        overrides.insert(
            "help".to_string(),
            vec![KeyBinding::new(KeyCode::Char('h'), KeyModifiers::NONE)],
        );
        let keymap = Keymap::default_keymap().with_overrides(&overrides).unwrap();
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));

        let line = text(footer_line(&state, &keymap, ActivePanel::Stacks, 120));
        assert!(line.contains("h help"));
        assert!(!line.contains("? help"));
    }

    #[test]
    fn footer_shows_refresh_indicator_when_loading() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.refresh_in_flight = true;
        assert!(footer(&state, ActivePanel::Stacks, 120).contains("Refreshing stacks"));
    }

    #[test]
    fn footer_shows_dismissible_error() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.error = Some("auth required".to_string());
        let line = footer(&state, ActivePanel::Stacks, 120);
        assert!(line.contains("error: auth required"));
        assert!(line.contains("x dismiss"));
    }

    #[test]
    fn footer_shows_sync_progress() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.sync_in_flight = true;
        assert!(footer(&state, ActivePanel::Stacks, 120).contains("Syncing stack"));
    }
}
