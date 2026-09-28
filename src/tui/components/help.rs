//! The `?` help overlay: every key binding, grouped by what it acts on.
//!
//! Generated from the live [`Keymap`], so rebinding a key in `config.toml`
//! changes what's shown here, and it's where toggle states (submit
//! `--auto`/`--open`, sync `--prune`) are shown instead of the footer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::theme::ui::THEME;
use crate::tui::keymap::{KeyIntent, Keymap};
use crate::tui::state::AppState;
use crate::tui::widgets::{centered_rect, panel_block};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpSection {
    pub(crate) title: &'static str,
    /// `(keys, description)` pairs.
    pub(crate) rows: Vec<(String, String)>,
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

/// The overlay's contents for `keymap`, with current toggle states.
pub(crate) fn sections(keymap: &Keymap, state: &AppState) -> Vec<HelpSection> {
    use KeyIntent::*;
    let row = |intent: KeyIntent, description: String| (keymap.all_labels(intent), description);
    let rows = |entries: &[(KeyIntent, &str)]| {
        entries
            .iter()
            .map(|(intent, description)| row(*intent, description.to_string()))
            .collect::<Vec<_>>()
    };

    let mut diff = rows(&[
        (PageDown, "page down"),
        (PageUp, "page up"),
        (HalfPageDown, "half page down"),
        (HalfPageUp, "half page up"),
    ]);
    // `gg` is a two-key sequence handled by the diff panel, not the keymap.
    diff.push(("gg".to_string(), "top".to_string()));
    diff.push(row(End, "bottom".to_string()));

    vec![
        HelpSection {
            title: "Navigation",
            rows: rows(&[
                (MoveDown, "move down"),
                (MoveUp, "move up"),
                (FocusNext, "next panel"),
                (FocusPrevious, "previous panel"),
                (DrillIn, "open / drill in"),
                (Back, "back / quit"),
                (Refresh, "refresh"),
            ]),
        },
        HelpSection {
            title: "Stack",
            rows: vec![
                row(Checkout, "check out stack or layer".to_string()),
                row(AddLayer, "add a layer".to_string()),
                row(Submit, "submit stack".to_string()),
                row(
                    ToggleSubmitAuto,
                    format!("submit --auto: {}", on_off(state.submit_options.auto)),
                ),
                row(
                    ToggleSubmitOpen,
                    format!("submit --open: {}", on_off(state.submit_options.open)),
                ),
                row(Sync, "sync stack".to_string()),
                row(
                    ToggleSyncPrune,
                    format!("sync --prune: {}", on_off(state.sync_options.prune)),
                ),
                row(Unstack, "unstack (local only)".to_string()),
                row(UnstackRemote, "unstack on GitHub".to_string()),
            ],
        },
        HelpSection {
            title: "Layer",
            rows: rows(&[
                (OpenExternal, "open PR in browser"),
                (ToggleDiff, "toggle diff view"),
            ]),
        },
        HelpSection {
            title: "Diff",
            rows: diff,
        },
        HelpSection {
            title: "General",
            rows: rows(&[
                (DismissMessage, "dismiss error or notice"),
                (Help, "toggle this help"),
            ]),
        },
    ]
}

fn section_lines(section: &HelpSection) -> Vec<Line<'static>> {
    let key_width = section
        .rows
        .iter()
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines = vec![Line::from(Span::styled(section.title, THEME.text.heading))];
    lines.extend(section.rows.iter().map(|(keys, description)| {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{keys:<key_width$}"), THEME.text.key),
            Span::raw("  "),
            Span::raw(description.clone()),
        ])
    }));
    lines
}

/// `sections` stacked with a blank line between each.
fn column_lines(sections: &[&HelpSection]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.extend(section_lines(section));
    }
    lines
}

/// Below this inner width the overlay falls back to a single column.
const TWO_COLUMN_MIN_WIDTH: u16 = 64;

/// Renders the overlay centered over `area`, in two columns when wide
/// enough, scrolled down by `scroll` lines (clamped so the last line stays
/// at the bottom). Returns the largest useful scroll offset.
pub(crate) fn render(
    frame: &mut Frame,
    area: Rect,
    keymap: &Keymap,
    state: &AppState,
    scroll: u16,
) -> u16 {
    let popup = centered_rect(area, 90, 90);
    frame.render_widget(Clear, popup);

    let close = keymap.short_label(KeyIntent::Help).unwrap_or_default();
    let block = panel_block(format!("keybindings ({close}/esc close, j/k scroll)"), true);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let sections = sections(keymap, state);
    let (left, right): (Vec<&HelpSection>, Vec<&HelpSection>) = sections
        .iter()
        .partition(|section| matches!(section.title, "Navigation" | "Diff"));

    let columns: Vec<(Vec<Line>, Rect)> = if inner.width >= TWO_COLUMN_MIN_WIDTH {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(inner);
        vec![
            (column_lines(&left), left_area),
            (column_lines(&right), right_area),
        ]
    } else {
        let all: Vec<&HelpSection> = sections.iter().collect();
        vec![(column_lines(&all), inner)]
    };

    let tallest = columns
        .iter()
        .map(|(lines, _)| lines.len())
        .max()
        .unwrap_or(0);
    let max_scroll =
        u16::try_from(tallest.saturating_sub(usize::from(inner.height))).unwrap_or(u16::MAX);
    let scroll = scroll.min(max_scroll);
    for (lines, column) in columns {
        frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), column);
    }
    max_scroll
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keymap::KeyBinding;
    use crate::test_fixtures::{app_state, stack_summary};
    use crate::tui::state::Screen;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::BTreeMap;

    fn find<'a>(sections: &'a [HelpSection], description: &str) -> &'a (String, String) {
        sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|(_, d)| d.starts_with(description))
            .unwrap_or_else(|| panic!("no row {description:?}"))
    }

    #[test]
    fn lists_every_binding_for_an_intent() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let sections = sections(&Keymap::default_keymap(), &state);
        assert_eq!(find(&sections, "move down").0, "down/j");
        assert_eq!(find(&sections, "half page down").0, "ctrl+d");
        assert_eq!(find(&sections, "unstack on GitHub").0, "U");
    }

    #[test]
    fn shows_current_toggle_states() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.submit_options.auto = true;
        let sections = sections(&Keymap::default_keymap(), &state);
        assert_eq!(find(&sections, "submit --auto").1, "submit --auto: on");
        assert_eq!(find(&sections, "submit --open").1, "submit --open: off");
        assert_eq!(find(&sections, "sync --prune").1, "sync --prune: off");
    }

    #[test]
    fn follows_rebound_keys() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "sync".to_string(),
            vec![KeyBinding::new(KeyCode::Char('y'), KeyModifiers::CONTROL)],
        );
        let keymap = Keymap::default_keymap().with_overrides(&overrides).unwrap();
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        assert_eq!(find(&sections(&keymap, &state), "sync stack").0, "ctrl+y");
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn every_section_fits_without_scrolling_at_80_by_24() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut max_scroll = u16::MAX;
        terminal
            .draw(|frame| {
                max_scroll = render(frame, frame.area(), &Keymap::default_keymap(), &state, 0);
            })
            .unwrap();
        let screen = screen_text(&terminal);
        assert_eq!(max_scroll, 0);
        for title in [
            "Navigation",
            "Stack",
            "Layer",
            "Diff",
            "General",
            "toggle this help",
        ] {
            assert!(screen.contains(title), "missing {title:?}");
        }
    }

    #[test]
    fn scrolls_to_reach_the_last_row_when_too_short() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let mut terminal = Terminal::new(TestBackend::new(50, 14)).unwrap();
        let mut max_scroll = 0;
        terminal
            .draw(|frame| {
                max_scroll = render(frame, frame.area(), &Keymap::default_keymap(), &state, 0);
            })
            .unwrap();
        assert!(max_scroll > 0);
        assert!(!screen_text(&terminal).contains("toggle this help"));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &Keymap::default_keymap(),
                    &state,
                    u16::MAX,
                );
            })
            .unwrap();
        assert!(screen_text(&terminal).contains("toggle this help"));
    }

    #[test]
    fn renders_at_small_and_large_sizes() {
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        for (width, height) in [(60, 20), (160, 45)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    render(frame, frame.area(), &Keymap::default_keymap(), &state, 0);
                })
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(screen.contains("keybindings"));
            assert!(screen.contains("Navigation"));
        }
    }
}
