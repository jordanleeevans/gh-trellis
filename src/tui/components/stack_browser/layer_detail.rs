//! The selected layer's PR summary, with its changed files (or, when the
//! diff panel is focused, the full diff) below.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::stack::{Layer, LayerDetail, StackSummary};
use crate::theme::ui::THEME;
use crate::tui::state::{AppState, layer_detail_cache_key, layer_diff_cache_key};
use crate::tui::widgets::{glyphs, panel_block};

use super::diff::{render_diff, render_diff_files};
use super::layout::summary_height;
use super::navigator::layer_title;
use super::{ActivePanel, PanelView};

pub(super) fn render_layer_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    stack: &StackSummary,
    selected: Option<usize>,
    view: PanelView,
) {
    let active_panel = view.active_panel;
    let Some(selected) = selected else {
        frame.render_widget(
            Paragraph::new("No layer selected")
                .style(THEME.text.muted)
                .block(panel_block("details", active_panel == ActivePanel::Detail)),
            area,
        );
        return;
    };

    let Some(layer) = stack.layers.get(selected) else {
        return;
    };

    let rebase_status = if layer.needs_rebase {
        Span::styled(
            "Needs rebase",
            Style::default()
                .fg(THEME.colors.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "Up to date",
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD),
        )
    };

    let detail = state
        .layer_details
        .get(&layer_detail_cache_key(stack, layer));
    let diff_key = layer_diff_cache_key(stack, layer);
    let diff = state.layer_diffs.get(&diff_key).map(String::as_str);
    let diff_loading = state.layer_diffs.is_loading(&diff_key);

    if active_panel == ActivePanel::Diff {
        render_diff(frame, area, stack, selected, diff, diff_loading, view);
        return;
    }

    let lines = detail_lines(layer, rebase_status, detail);
    let [summary_area, files_area] = Layout::vertical([
        Constraint::Length(summary_height(area.height)),
        Constraint::Min(0),
    ])
    .areas(area);

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(
                layer_title(layer),
                active_panel == ActivePanel::Detail,
            ))
            .wrap(Wrap { trim: false }),
        summary_area,
    );

    render_diff_files(
        frame,
        files_area,
        diff,
        diff_loading,
        active_panel == ActivePanel::Files,
        view.selected_diff_file,
    );
}

pub(super) fn detail_lines(
    layer: &Layer,
    rebase_status: Span<'static>,
    detail: Option<&LayerDetail>,
) -> Vec<Line<'static>> {
    let pr = layer.pull_request.as_ref();
    let title = detail
        .map(|detail| detail.pull_request.title.clone())
        .filter(|title| !title.is_empty())
        .or_else(|| pr.and_then(|pr| pr.title.clone()))
        .unwrap_or_else(|| "Not submitted".to_string());
    let author = detail
        .and_then(|detail| detail.commits.first())
        .and_then(|commit| commit.author.clone())
        .unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        Line::from(vec![Span::styled(
            pr.map(|pr| format!("PR #{}", pr.number))
                .unwrap_or_else(|| "No PR".to_string()),
            THEME.text.heading,
        )]),
        Line::from(Span::styled(title, THEME.text.body)),
        Line::from(vec![
            label_span("status"),
            rebase_status,
            Span::raw("  "),
            checks_span(detail),
        ]),
        Line::from(""),
        labeled_line("branch", layer.branch.clone()),
        labeled_line("base", layer.base.clone()),
        labeled_line("author", author),
    ];

    if let Some(detail) = detail
        && let Some(snippet) = &detail.pull_request.description_snippet
    {
        lines.push(labeled_line("summary", snippet.clone()));
    }

    lines
}

pub(super) fn labeled_line(label: &str, value: String) -> Line<'static> {
    Line::from(vec![label_span(label), Span::raw(value)])
}

pub(super) fn label_span(label: &str) -> Span<'static> {
    Span::styled(format!("{label:<10}"), THEME.text.label)
}

pub(super) fn checks_span(detail: Option<&LayerDetail>) -> Span<'static> {
    let Some(detail) = detail else {
        return Span::styled("loading details", THEME.text.muted);
    };

    let checks = detail.pull_request.checks;
    if checks.total == 0 {
        Span::styled("no checks", THEME.text.muted)
    } else if checks.failing > 0 {
        Span::styled(
            format!(
                "{} {} failing, {} pending of {}",
                glyphs().cross,
                checks.failing,
                checks.pending,
                checks.total
            ),
            Style::default().fg(THEME.colors.danger),
        )
    } else if checks.pending > 0 {
        Span::styled(
            format!(
                "{} {} passing, {} pending of {}",
                glyphs().pending,
                checks.passing,
                checks.pending,
                checks.total
            ),
            Style::default().fg(THEME.colors.warning),
        )
    } else {
        Span::styled(
            format!(
                "{} {}/{} checks passing",
                glyphs().check,
                checks.passing,
                checks.total
            ),
            Style::default().fg(THEME.colors.success),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::{
        CheckSummary, LayerCommit, PullRequestDetail, PullRequestRef, ReviewerState,
    };

    #[test]
    fn detail_lines_include_cached_layer_detail() {
        let mut layer = crate::test_fixtures::layer("feature/layer-1");
        layer.pull_request = Some(PullRequestRef {
            number: 42,
            url: "https://example.test/pull/42".to_string(),
            state: "OPEN".to_string(),
            title: None,
            is_draft: None,
            checks_status: None,
            review_decision: None,
        });
        let detail = LayerDetail {
            commits: vec![LayerCommit {
                oid: "abcdef123456".to_string(),
                subject: "feat: render details".to_string(),
                author: Some("john-doe".to_string()),
                authored_at: "2026-09-18T10:00:00Z".to_string(),
            }],
            pull_request: PullRequestDetail {
                title: "Layer detail pane".to_string(),
                description_snippet: Some("Shows the selected layer.".to_string()),
                reviewers: vec![ReviewerState {
                    login: "octocat".to_string(),
                    state: "APPROVED".to_string(),
                }],
                checks: CheckSummary {
                    total: 2,
                    passing: 1,
                    failing: 0,
                    pending: 1,
                },
                labels: vec!["tui".to_string()],
            },
        };

        let text = detail_lines(&layer, Span::raw("Up to date"), Some(&detail))
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Layer detail pane"));
        assert!(text.contains("Shows the selected layer."));
        assert!(text.contains("Shows the selected layer."));
        assert!(text.contains("PR #42"));
        assert!(text.contains("john-doe"));
        assert!(text.contains("1 passing, 1 pending of 2"));
    }
}
