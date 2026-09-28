use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::theme::ui::THEME;

pub(crate) fn panel_block(title: impl Into<String>, active: bool) -> Block<'static> {
    let title = panel_title(title, active);

    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(THEME.border(active))
}

fn panel_title(title: impl Into<String>, active: bool) -> Line<'static> {
    let title = title.into();
    let (label, title_style) = if active {
        (format!(" > {title} < "), THEME.text.active_title)
    } else {
        (format!(" {title} "), THEME.text.inactive_title)
    };

    Line::from(Span::styled(label, title_style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_panel_title_uses_focus_markers() {
        let title = panel_title("files", true)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(title.contains("> files <"));
    }
}
