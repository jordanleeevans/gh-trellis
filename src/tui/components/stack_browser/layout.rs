//! Where each part of the browser goes for a given terminal size and focus.
//!
//! Pure geometry: nothing here draws. The layout is recomputed from the
//! frame area on every draw, so a terminal resize takes effect on the next
//! frame with no cached sizes.

use ratatui::layout::{Constraint, Layout, Rect};

use super::ActivePanel;

/// Below this width the navigator and detail columns no longer both fit, so
/// only one is shown at a time. A layer row in the navigator needs 30
/// columns plus its border (`  ├─ name(14) #123 ●`), and the detail side
/// needs about 58 for file paths and the PR summary. 90 leaves both at
/// their minimum.
pub(super) const SINGLE_COLUMN_BELOW: u16 = 90;

/// Fewer rows than this and the 3-row bordered header shrinks to one row.
/// 28 rows leaves 80x24 and 60x20 with room for the detail panels.
pub(super) const COMPACT_HEADER_BELOW: u16 = 28;

/// The navigator's share of the width, with a floor so its rows never clip
/// between the single-column threshold and about 100 columns.
const NAVIGATOR_PERCENT: u16 = 32;
const NAVIGATOR_MIN_WIDTH: u16 = 32;

const HEADER_ROWS: u16 = 3;
const COMPACT_HEADER_ROWS: u16 = 1;
const FOOTER_ROWS: u16 = 2;

/// Rows the PR summary wants (7 lines, a snippet line and two borders).
const SUMMARY_ROWS: u16 = 10;
/// Rows the changed-files list keeps before the summary gives ground.
const MIN_FILES_ROWS: u16 = 6;
const MIN_SUMMARY_ROWS: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BrowserLayout {
    pub header: Rect,
    pub content: Rect,
    pub footer: Rect,
    /// The navigator, when visible. Absent in single-column mode while a
    /// detail panel is focused.
    pub navigator: Option<Rect>,
    /// The detail column, when visible. Absent in single-column mode while
    /// the navigator is focused.
    pub detail: Option<Rect>,
    pub compact_header: bool,
    pub single_column: bool,
}

impl BrowserLayout {
    /// Which of the two views is showing, for the header. `None` when both
    /// columns are visible.
    pub fn view_label(&self) -> Option<&'static str> {
        match (self.single_column, self.navigator.is_some()) {
            (false, _) => None,
            (true, true) => Some("1/2 navigator"),
            (true, false) => Some("2/2 details"),
        }
    }
}

pub(super) fn compute(area: Rect, focus: ActivePanel) -> BrowserLayout {
    let compact_header = area.height < COMPACT_HEADER_BELOW;
    let header_rows = if compact_header {
        COMPACT_HEADER_ROWS
    } else {
        HEADER_ROWS
    };
    let [header, content, footer] = Layout::vertical([
        Constraint::Length(header_rows),
        Constraint::Min(0),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(area);

    let single_column = area.width < SINGLE_COLUMN_BELOW;
    let (navigator, detail) = if single_column {
        if matches!(focus, ActivePanel::Stacks | ActivePanel::Layers) {
            (Some(content), None)
        } else {
            (None, Some(content))
        }
    } else {
        let nav_width = (content.width * NAVIGATOR_PERCENT / 100).max(NAVIGATOR_MIN_WIDTH);
        let [nav, detail] =
            Layout::horizontal([Constraint::Length(nav_width), Constraint::Min(0)]).areas(content);
        (Some(nav), Some(detail))
    };

    BrowserLayout {
        header,
        content,
        footer,
        navigator,
        detail,
        compact_header,
        single_column,
    }
}

/// Height of the PR summary panel in a detail column `height` rows tall:
/// full size when there is room, otherwise it gives rows to the file list.
pub(super) fn summary_height(height: u16) -> u16 {
    SUMMARY_ROWS
        .min(height.saturating_sub(MIN_FILES_ROWS))
        .max(MIN_SUMMARY_ROWS)
}

/// The scroll offset that keeps `focus_line` inside a bordered panel of
/// `outer_height` rows.
pub(super) fn scroll_to_show(focus_line: usize, outer_height: u16) -> u16 {
    let inner = usize::from(outer_height.saturating_sub(2)).max(1);
    u16::try_from(focus_line.saturating_sub(inner - 1)).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn wide_terminals_show_both_columns() {
        let layout = compute(area(160, 45), ActivePanel::Diff);
        assert!(!layout.single_column && !layout.compact_header);
        let (nav, detail) = (layout.navigator.unwrap(), layout.detail.unwrap());
        assert_eq!(nav.width + detail.width, 160);
        assert_eq!(layout.view_label(), None);
    }

    #[test]
    fn navigator_keeps_its_minimum_width_at_the_threshold() {
        let layout = compute(area(SINGLE_COLUMN_BELOW, 30), ActivePanel::Stacks);
        assert!(!layout.single_column);
        assert!(layout.navigator.unwrap().width >= NAVIGATOR_MIN_WIDTH);
        assert!(layout.detail.unwrap().width >= 58);
    }

    #[test]
    fn narrow_terminals_follow_focus() {
        for panel in [ActivePanel::Stacks, ActivePanel::Layers] {
            let layout = compute(area(60, 20), panel);
            assert_eq!(layout.navigator, Some(layout.content));
            assert_eq!(layout.detail, None);
            assert_eq!(layout.view_label(), Some("1/2 navigator"));
        }
        for panel in [ActivePanel::Detail, ActivePanel::Files, ActivePanel::Diff] {
            let layout = compute(area(60, 20), panel);
            assert_eq!(layout.navigator, None);
            assert_eq!(layout.detail, Some(layout.content));
            assert_eq!(layout.view_label(), Some("2/2 details"));
        }
    }

    #[test]
    fn short_terminals_use_a_one_row_header() {
        let short = compute(area(80, 24), ActivePanel::Stacks);
        assert_eq!(short.header.height, 1);
        let tall = compute(area(80, 40), ActivePanel::Stacks);
        assert_eq!(tall.header.height, 3);
        assert_eq!(short.footer.height, 2);
    }

    #[test]
    fn rows_are_conserved_and_tiny_areas_do_not_panic() {
        for (w, h) in [(60, 20), (80, 24), (100, 30), (160, 45), (10, 3), (0, 0)] {
            let l = compute(area(w, h), ActivePanel::Files);
            assert!(l.header.height + l.content.height + l.footer.height <= h);
        }
    }

    #[test]
    fn summary_gives_rows_to_the_file_list() {
        assert_eq!(summary_height(40), 10);
        assert_eq!(summary_height(16), 10);
        assert_eq!(summary_height(13), 7);
        assert_eq!(summary_height(5), MIN_SUMMARY_ROWS);
    }

    #[test]
    fn scroll_keeps_the_focus_line_visible() {
        assert_eq!(scroll_to_show(0, 10), 0);
        assert_eq!(scroll_to_show(7, 10), 0);
        assert_eq!(scroll_to_show(8, 10), 1);
        assert_eq!(scroll_to_show(20, 2), 20);
    }
}
