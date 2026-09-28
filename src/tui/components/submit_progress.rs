//! The submit progress overlay: a gauge plus one line per layer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap};

use crate::theme::ui::THEME;
use crate::tui::state::submit_progress::{LayerSubmitStatus, SubmitLayerProgress, SubmitProgress};
use crate::tui::widgets::{centered_rect, glyphs};

pub(crate) fn render(frame: &mut Frame, area: Rect, progress: &SubmitProgress) {
    let area = centered_rect(area, 70, 60);

    let failed = progress.failed_count();
    let title = if !progress.finished {
        " submitting stack ".to_string()
    } else if failed > 0 {
        format!(" submit finished — {failed} failed ")
    } else {
        " submit finished ".to_string()
    };
    let border_color = if failed > 0 {
        THEME.colors.danger
    } else if progress.finished {
        THEME.colors.success
    } else {
        THEME.colors.primary
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [gauge_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    let gauge_color = if failed > 0 {
        THEME.colors.danger
    } else if progress.finished {
        THEME.colors.success
    } else {
        THEME.colors.warning
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color).bg(THEME.colors.surface))
        .label(format!(
            "{}/{} layers",
            progress.completed_count(),
            progress.total()
        ))
        .ratio(progress.ratio().clamp(0.0, 1.0));
    frame.render_widget(gauge, gauge_area);

    let lines = progress
        .layers
        .iter()
        .map(submit_layer_line)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list_area);

    let footer_text = if !progress.finished {
        "submitting… please wait"
    } else if failed > 0 {
        "some layers failed — esc/q to dismiss"
    } else {
        "all layers submitted — esc/q to dismiss"
    };
    frame.render_widget(
        Paragraph::new(footer_text).style(THEME.text.muted),
        footer_area,
    );
}

fn submit_layer_line(layer: &SubmitLayerProgress) -> Line<'static> {
    let (glyph, style, text) = match &layer.status {
        LayerSubmitStatus::Pending => (glyphs().pending, THEME.text.muted, "pending".to_string()),
        LayerSubmitStatus::InProgress => (
            glyphs().running,
            Style::default().fg(THEME.colors.warning),
            "pushing…".to_string(),
        ),
        LayerSubmitStatus::Pushed => (
            glyphs().running,
            Style::default().fg(THEME.colors.warning),
            "pushed, syncing pull request…".to_string(),
        ),
        LayerSubmitStatus::PullRequestCreated { number } => (
            glyphs().check,
            Style::default().fg(THEME.colors.success),
            format!("pushed, PR #{number} created"),
        ),
        LayerSubmitStatus::PullRequestUpdated { number } => (
            glyphs().check,
            Style::default().fg(THEME.colors.success),
            format!("pushed, PR #{number} updated"),
        ),
        LayerSubmitStatus::Failed(message) => (
            glyphs().cross,
            Style::default().fg(THEME.colors.danger),
            format!("failed: {message}"),
        ),
    };

    Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{:<28}", layer.branch), style),
        Span::styled(text, style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_progress_reports_partial_failure_per_layer_in_rendered_lines() {
        let mut progress =
            SubmitProgress::new(0, vec!["a-layer-0".to_string(), "a-layer-1".to_string()]);
        progress.set_status(0, LayerSubmitStatus::PullRequestCreated { number: 10 });
        progress.set_status(1, LayerSubmitStatus::Failed("push rejected".to_string()));
        progress.finished = true;

        let lines = progress
            .layers
            .iter()
            .map(submit_layer_line)
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(lines[0].contains("PR #10 created"));
        assert!(lines[1].contains("failed: push rejected"));
    }
}
