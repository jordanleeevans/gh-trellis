//! Parsing a layer's unified diff into files, and rendering the changed
//! files tree and the diff itself.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::stack::StackSummary;
use crate::theme::ui::THEME;
use crate::tui::state::lower_layer_ref;
use crate::tui::widgets::{glyphs, panel_block};

use super::{ActivePanel, PanelView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiffFile {
    path: String,
    status: char,
    additions: usize,
    deletions: usize,
    start: usize,
    end: usize,
}

pub(super) fn parse_diff_files(diff: &str) -> Vec<DiffFile> {
    let lines: Vec<&str> = diff.lines().collect();
    let mut files: Vec<DiffFile> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if let Some(path) = line.strip_prefix("diff --git a/") {
            if let Some(previous) = files.last_mut() {
                previous.end = index;
            }

            let path = path.split(" b/").nth(1).unwrap_or(path).to_string();
            files.push(DiffFile {
                path,
                status: 'M',
                additions: 0,
                deletions: 0,
                start: index,
                end: lines.len(),
            });
        } else if let Some(file) = files.last_mut() {
            if line.starts_with("new file mode") {
                file.status = 'A';
            } else if line.starts_with("deleted file mode") {
                file.status = 'D';
            } else if line.starts_with("rename from") || line.starts_with("rename to") {
                file.status = 'R';
            } else if line.starts_with('+') && !line.starts_with("+++") {
                file.additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                file.deletions += 1;
            }
        }
    }

    files
}

pub(super) fn render_diff_files(
    frame: &mut Frame,
    area: Rect,
    diff: Option<&str>,
    is_loading: bool,
    is_active: bool,
    selected_diff_file: usize,
) {
    let block = panel_block("changed files", is_active);

    let Some(diff) = diff else {
        let message = if is_loading {
            "Loading diff..."
        } else {
            "Diff not loaded."
        };
        frame.render_widget(
            Paragraph::new(message).style(THEME.text.muted).block(block),
            area,
        );
        return;
    };

    let files = parse_diff_files(diff);
    if files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes in this layer.")
                .style(THEME.text.muted)
                .block(block),
            area,
        );
        return;
    }

    let lines = build_file_tree_lines(&files, area.width, selected_diff_file, is_active);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(super) fn render_diff(
    frame: &mut Frame,
    area: Rect,
    stack: &StackSummary,
    selected_layer: usize,
    diff: Option<&str>,
    is_loading: bool,
    view: PanelView,
) {
    let is_active = view.active_panel == ActivePanel::Diff;
    let selected_diff_file = view.selected_diff_file;
    let diff_scroll = view.diff_scroll;
    let Some(layer) = stack.layers.get(selected_layer) else {
        return;
    };
    let files = diff.map(parse_diff_files).unwrap_or_default();
    let selected_file = files.get(selected_diff_file.min(files.len().saturating_sub(1)));
    let title = selected_file
        .map(|file| format!(" diff {} ", file.path))
        .unwrap_or_else(|| {
            format!(
                " diff {}..{} ",
                lower_layer_ref(stack, selected_layer),
                layer.branch
            )
        });
    let block = panel_block(title, is_active);

    let Some(diff) = diff else {
        let message = if is_loading {
            "Loading diff..."
        } else {
            "Diff unavailable."
        };
        frame.render_widget(
            Paragraph::new(message).style(THEME.text.muted).block(block),
            area,
        );
        return;
    };

    let lines = diff.lines().collect::<Vec<_>>();
    let visible_height = area.height.saturating_sub(2) as usize;
    let (line_start, line_end) = selected_file
        .map(|file| (file.start, file.end))
        .unwrap_or((0, lines.len()));
    let max_start = line_end.saturating_sub(visible_height);
    let start =
        (line_start + usize::from(diff_scroll)).clamp(line_start, max_start.max(line_start));
    let end = (start + visible_height).min(line_end);
    let rendered = lines[start..end]
        .iter()
        .map(|line| diff_line(line))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(rendered).block(block), area);
}

pub(super) fn diff_line(text: &str) -> Line<'static> {
    let style = if text.starts_with("+++") || text.starts_with("---") {
        Style::default().fg(THEME.colors.text_muted)
    } else if text.starts_with('+') {
        Style::default().fg(THEME.colors.success)
    } else if text.starts_with('-') {
        Style::default().fg(THEME.colors.danger)
    } else if text.starts_with("@@") {
        Style::default()
            .fg(THEME.colors.primary)
            .add_modifier(Modifier::BOLD)
    } else if text.starts_with("diff --git") {
        Style::default()
            .fg(THEME.colors.secondary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Line::from(Span::styled(text.to_string(), style))
}

pub(super) fn build_file_tree_lines(
    files: &[DiffFile],
    width: u16,
    selected_index: usize,
    is_active: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut previous_dirs: Vec<&str> = Vec::new();

    for (file_index, file) in files.iter().enumerate() {
        let parts = file.path.split('/').collect::<Vec<_>>();
        let dirs = &parts[..parts.len().saturating_sub(1)];
        let common_prefix = previous_dirs
            .iter()
            .zip(dirs.iter())
            .take_while(|(left, right)| left == right)
            .count();

        for (depth, dir) in dirs.iter().enumerate().skip(common_prefix) {
            lines.push(folder_line(dir, depth, depth + 1 == dirs.len()));
        }

        lines.push(diff_file_line(
            file,
            width,
            dirs.len(),
            file_index == selected_index,
            is_active,
        ));
        previous_dirs = dirs.to_vec();
    }

    lines
}

pub(super) fn folder_line(name: &str, depth: usize, is_leaf: bool) -> Line<'static> {
    let branch = if is_leaf { "└─" } else { "├─" };
    Line::from(vec![
        Span::raw(format!("{}{} ", "  ".repeat(depth), branch)),
        Span::styled(glyphs().folder_open, THEME.text.muted),
        Span::raw(" "),
        Span::styled(name.to_string(), THEME.text.muted),
    ])
}

pub(super) fn diff_file_line(
    file: &DiffFile,
    width: u16,
    depth: usize,
    is_selected: bool,
    is_active: bool,
) -> Line<'static> {
    let stats = format!("+{}  -{}", file.additions, file.deletions);
    let branch = "└─";
    let prefix = format!("{}{} {} ", "  ".repeat(depth), branch, glyphs().file);
    let available = width.saturating_sub(4) as usize;
    let stats_len = stats.chars().count();
    let gap = 2usize;
    let max_path_len = available.saturating_sub(prefix.chars().count() + stats_len + gap);
    let file_name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(file.path.as_str())
        .to_string();
    let path = truncate_text(&file_name, max_path_len.max(1));
    let spacer = " ".repeat(
        available.saturating_sub(prefix.chars().count() + path.chars().count() + stats_len),
    );
    let style = if is_selected && is_active {
        THEME.text.selected
    } else {
        Style::default()
    };

    Line::from(vec![
        Span::styled(prefix, style),
        Span::styled(path, style),
        Span::styled(spacer, style),
        Span::styled(stats, THEME.text.muted),
    ])
}

pub(super) fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    if max_len <= 1 {
        return "…".to_string();
    }

    let mut truncated = text.chars().take(max_len - 1).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_files_finds_each_file_range() {
        let diff = [
            "diff --git a/src/a.rs b/src/a.rs",
            "index 123..456 100644",
            "--- a/src/a.rs",
            "+++ b/src/a.rs",
            "@@ -1 +1 @@",
            "-old",
            "+new",
            "diff --git a/src/b.rs b/src/b.rs",
            "@@ -3 +3 @@",
            "+more",
        ]
        .join("\n");

        let files = parse_diff_files(&diff);

        assert_eq!(
            files,
            vec![
                DiffFile {
                    path: "src/a.rs".to_string(),
                    status: 'M',
                    additions: 1,
                    deletions: 1,
                    start: 0,
                    end: 7,
                },
                DiffFile {
                    path: "src/b.rs".to_string(),
                    status: 'M',
                    additions: 1,
                    deletions: 0,
                    start: 7,
                    end: 10,
                },
            ]
        );
    }

    #[test]
    fn parse_diff_files_detects_added_file() {
        let diff = [
            "diff --git a/src/new.rs b/src/new.rs",
            "new file mode 100644",
            "--- /dev/null",
            "+++ b/src/new.rs",
            "+new",
        ]
        .join("\n");

        let files = parse_diff_files(&diff);
        assert_eq!(files[0].status, 'A');
        assert_eq!(files[0].additions, 1);
    }
}
