//! The inline `gh stack add` prompt overlay.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::theme::ui::THEME;
use crate::tui::action::Action;
use crate::tui::widgets::{centered_rect_min, panel_block};

/// Which field of the "add layer" prompt is currently being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddLayerField {
    Branch,
    Message,
}

/// Inline prompt state for `gh stack add`, opened by
/// [`crate::tui::keymap::KeyIntent::AddLayer`].
///
/// The flow is two sequential single-line fields: the new branch name first,
/// then an optional commit message (`-m`). Enter on the branch field moves
/// to the message field (only once the branch name is non-empty); Enter on
/// the message field submits an [`Action::AddLayer`] (with `message: None`
/// if left blank) and closes the prompt. Escape cancels at either step
/// without dispatching anything.
#[derive(Debug, Clone)]
pub(crate) struct AddLayerPrompt {
    pub(crate) stack_index: usize,
    pub(crate) field: AddLayerField,
    pub(crate) branch: String,
    pub(crate) message: String,
}

/// What a key press did to the prompt.
#[derive(Debug)]
pub(crate) enum PromptOutcome {
    /// Still typing; keep the prompt open.
    Editing,
    /// Escape: close without dispatching anything.
    Cancelled,
    /// Enter on the last field: close and dispatch this action.
    Submitted(Action),
}

impl AddLayerPrompt {
    pub(crate) fn new(stack_index: usize) -> Self {
        Self {
            stack_index,
            field: AddLayerField::Branch,
            branch: String::new(),
            message: String::new(),
        }
    }

    /// Handles a key while the prompt is open. Every printable key is text
    /// input here, including letters that are otherwise global shortcuts
    /// (like `q` or `c`), so they're never mistaken for a `KeyIntent`.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> PromptOutcome {
        match key.code {
            KeyCode::Esc => return PromptOutcome::Cancelled,
            KeyCode::Enter => match self.field {
                AddLayerField::Branch => {
                    if !self.branch.trim().is_empty() {
                        self.field = AddLayerField::Message;
                    }
                }
                AddLayerField::Message => {
                    let message = self.message.trim();
                    return PromptOutcome::Submitted(Action::AddLayer {
                        stack_index: self.stack_index,
                        branch: self.branch.trim().to_string(),
                        message: (!message.is_empty()).then(|| message.to_string()),
                    });
                }
            },
            KeyCode::Backspace => {
                self.active_field().pop();
            }
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
                self.active_field().push(c);
            }
            _ => {}
        }

        PromptOutcome::Editing
    }

    fn active_field(&mut self) -> &mut String {
        match self.field {
            AddLayerField::Branch => &mut self.branch,
            AddLayerField::Message => &mut self.message,
        }
    }
}

/// Renders the prompt as a small bordered overlay centered over `area`.
pub(crate) fn render(frame: &mut Frame, area: Rect, prompt: &AddLayerPrompt) {
    let popup_area = centered_rect_min(area, 60, 8, 44, 8);
    frame.render_widget(Clear, popup_area);

    let (label, value, hint) = match prompt.field {
        AddLayerField::Branch => (
            "New branch name",
            prompt.branch.as_str(),
            "enter continue  esc cancel",
        ),
        AddLayerField::Message => (
            "Commit message (optional, -m)",
            prompt.message.as_str(),
            "enter add layer  esc cancel",
        ),
    };

    let lines = vec![
        Line::from(Span::styled(label, THEME.text.label)),
        Line::from(Span::raw(format!("{value}\u{2588}"))),
        Line::from(""),
        Line::from(Span::styled(hint, THEME.text.muted)),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("add layer", true))
            .wrap(Wrap { trim: false }),
        popup_area,
    );
}
