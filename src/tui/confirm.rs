//! Shared confirmation-modal framework for destructive (and non-destructive)
//! actions that need an explicit "are you sure?" step before firing.
//!
//! Any caller builds a [`ConfirmModal`], attaches the [`Action`] that should
//! run once the user confirms, and hands it to [`AppState::confirm`] (via
//! `Action::ShowConfirm`). From there the shared key handling and rendering
//! in this module take over: Enter accepts, Esc cancels, and `danger: true`
//! modals additionally require the user to type a confirmation phrase before
//! Enter is honored, so a stray keystroke can never fire a destructive action.
//!
//! The phrase required for danger confirmations is fixed (see
//! [`DANGER_CONFIRM_PHRASE`]) rather than derived per-action, so every
//! danger modal in the app behaves identically and callers never need to
//! plumb a bespoke phrase through.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::theme::ui::THEME;

use super::app::Action;

/// The phrase a user must type (case-insensitively) to accept a `danger`
/// modal. Kept fixed and simple ("yes") rather than per-action so the
/// muscle-memory of confirming one danger action doesn't transfer to a
/// different one, while still being trivial to type under pressure.
pub const DANGER_CONFIRM_PHRASE: &str = "yes";

/// A generic confirmation modal reused by every destructive (or otherwise
/// confirmation-worthy) action in the app, rather than one-off modals per
/// call site.
#[derive(Debug, Clone)]
pub struct ConfirmModal {
    pub title: String,
    pub body: String,
    pub confirm_label: String,
    pub danger: bool,
    /// The action dispatched if the user confirms. `None` until a caller
    /// attaches one via [`ConfirmModal::on_confirm`]; accepting a modal with
    /// no action attached is treated as a no-op cancel rather than a panic,
    /// since a misconfigured caller should never be able to fire an
    /// arbitrary/undefined action.
    on_confirm: Option<Box<Action>>,
    /// What the user has typed so far, for `danger` modals only.
    typed_input: String,
}

impl ConfirmModal {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        confirm_label: impl Into<String>,
        danger: bool,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            confirm_label: confirm_label.into(),
            danger,
            on_confirm: None,
            typed_input: String::new(),
        }
    }

    /// Attaches the action to dispatch once the user confirms. Typical usage:
    /// `ConfirmModal::new(...).on_confirm(Action::SomeDestructiveThing)`.
    pub fn on_confirm(mut self, action: Action) -> Self {
        self.on_confirm = Some(Box::new(action));
        self
    }

    /// What the user currently has typed into the confirmation field.
    /// Only meaningful when `danger` is true.
    pub fn typed_input(&self) -> &str {
        &self.typed_input
    }

    /// Whether the modal is in a state where Enter should accept it: always
    /// true for non-danger modals, and only once the typed input matches the
    /// required phrase for danger modals.
    pub fn can_confirm(&self) -> bool {
        !self.danger
            || self
                .typed_input
                .trim()
                .eq_ignore_ascii_case(DANGER_CONFIRM_PHRASE)
    }

    pub(crate) fn push_char(&mut self, c: char) {
        self.typed_input.push(c);
    }

    pub(crate) fn pop_char(&mut self) {
        self.typed_input.pop();
    }

    /// Consumes the modal, returning the action to run if it can currently
    /// be confirmed, or `None` (leaving nothing to do) otherwise.
    pub(crate) fn into_confirmed_action(self) -> Option<Action> {
        if self.can_confirm() {
            self.on_confirm.map(|action| *action)
        } else {
            None
        }
    }
}

/// Translates a raw key event into the [`Action`]s a confirm modal should
/// produce. Called instead of the active screen's own key handling whenever
/// `AppState::confirm` is `Some`, so a modal always gets first refusal on
/// input: Esc cancels unconditionally, Enter accepts (subject to
/// [`ConfirmModal::can_confirm`]), and for `danger` modals plain characters
/// and backspace edit the typed confirmation phrase instead of doing
/// anything else.
pub(crate) fn handle_confirm_key(modal: &ConfirmModal, key: KeyEvent) -> Vec<Action> {
    match key.code {
        KeyCode::Esc => vec![Action::ConfirmCancel],
        KeyCode::Enter => vec![Action::ConfirmAccept],
        KeyCode::Backspace if modal.danger => vec![Action::ConfirmBackspace],
        KeyCode::Char(c) if modal.danger => vec![Action::ConfirmInput(c)],
        _ => Vec::new(),
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Draws the modal as a centered overlay on top of whatever else was drawn
/// this frame.
pub(crate) fn render(frame: &mut Frame, area: Rect, modal: &ConfirmModal) {
    let width = area.width.saturating_sub(4).clamp(24, 64);
    let extra_lines = if modal.danger { 3 } else { 0 };
    let height = (6 + extra_lines).min(area.height.saturating_sub(2)).max(6);
    let modal_area = centered_rect(width, height, area);

    frame.render_widget(Clear, modal_area);

    let accent = if modal.danger {
        THEME.colors.danger
    } else {
        THEME.colors.primary
    };

    let title_text = if modal.danger {
        format!(
            " {} {} ",
            crate::theme::glyphs::current().warning,
            modal.title
        )
    } else {
        format!(" {} ", modal.title)
    };

    let block = Block::default()
        .title(Line::from(Span::styled(
            title_text,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines = vec![
        Line::from(Span::styled(modal.body.clone(), THEME.text.body)),
        Line::from(""),
    ];

    if modal.danger {
        lines.push(Line::from(Span::styled(
            format!(
                "Type \"{}\" and press enter to confirm:",
                DANGER_CONFIRM_PHRASE
            ),
            Style::default()
                .fg(THEME.colors.warning)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("> ", THEME.text.label),
            Span::styled(format!("{}_", modal.typed_input()), THEME.text.body),
        ]));
        lines.push(Line::from(""));
    }

    let hint = if modal.danger {
        Line::from(vec![
            Span::styled("enter", THEME.text.key),
            Span::raw(format!(
                " {} (once \"{}\" is typed)  ",
                modal.confirm_label, DANGER_CONFIRM_PHRASE
            )),
            Span::styled("esc", THEME.text.key),
            Span::raw(" cancel"),
        ])
    } else {
        Line::from(vec![
            Span::styled("enter", THEME.text.key),
            Span::raw(format!(" {}  ", modal.confirm_label)),
            Span::styled("esc", THEME.text.key),
            Span::raw(" cancel"),
        ])
    };
    lines.push(hint);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn construction_defaults_to_no_typed_input_and_no_action() {
        let modal = ConfirmModal::new("Title", "Body", "Confirm", false);

        assert_eq!(modal.title, "Title");
        assert_eq!(modal.body, "Body");
        assert_eq!(modal.confirm_label, "Confirm");
        assert!(!modal.danger);
        assert_eq!(modal.typed_input(), "");
        assert!(modal.clone().into_confirmed_action().is_none());
    }

    #[test]
    fn non_danger_modal_can_confirm_immediately() {
        let modal =
            ConfirmModal::new("Quit?", "Really quit?", "Quit", false).on_confirm(Action::Quit);

        assert!(modal.can_confirm());
        assert!(matches!(modal.into_confirmed_action(), Some(Action::Quit)));
    }

    #[test]
    fn danger_modal_cannot_confirm_until_phrase_typed() {
        let mut modal = ConfirmModal::new("Merge?", "This merges the PR.", "Merge", true)
            .on_confirm(Action::Quit);

        assert!(!modal.can_confirm());
        assert!(modal.clone().into_confirmed_action().is_none());

        modal.push_char('n');
        modal.push_char('o');
        assert!(!modal.can_confirm());

        modal.pop_char();
        modal.pop_char();
        modal.push_char('y');
        modal.push_char('e');
        modal.push_char('s');
        assert!(modal.can_confirm());
        assert!(matches!(modal.into_confirmed_action(), Some(Action::Quit)));
    }

    #[test]
    fn danger_phrase_match_is_case_insensitive_and_trims_whitespace() {
        let mut modal = ConfirmModal::new("Merge?", "Body", "Merge", true);
        for c in " YES ".chars() {
            modal.push_char(c);
        }

        assert!(modal.can_confirm());
    }

    #[test]
    fn handle_confirm_key_esc_cancels_regardless_of_danger() {
        let modal = ConfirmModal::new("Quit?", "Body", "Quit", false);
        let actions = handle_confirm_key(&modal, key(KeyCode::Esc));
        assert!(matches!(actions.as_slice(), [Action::ConfirmCancel]));

        let danger_modal = ConfirmModal::new("Merge?", "Body", "Merge", true);
        let actions = handle_confirm_key(&danger_modal, key(KeyCode::Esc));
        assert!(matches!(actions.as_slice(), [Action::ConfirmCancel]));
    }

    #[test]
    fn handle_confirm_key_enter_accepts() {
        let modal = ConfirmModal::new("Quit?", "Body", "Quit", false);
        let actions = handle_confirm_key(&modal, key(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::ConfirmAccept]));
    }

    #[test]
    fn handle_confirm_key_types_into_danger_modal_only() {
        let danger_modal = ConfirmModal::new("Merge?", "Body", "Merge", true);
        let actions = handle_confirm_key(&danger_modal, key(KeyCode::Char('y')));
        assert!(matches!(actions.as_slice(), [Action::ConfirmInput('y')]));

        let actions = handle_confirm_key(&danger_modal, key(KeyCode::Backspace));
        assert!(matches!(actions.as_slice(), [Action::ConfirmBackspace]));

        let non_danger_modal = ConfirmModal::new("Quit?", "Body", "Quit", false);
        let actions = handle_confirm_key(&non_danger_modal, key(KeyCode::Char('y')));
        assert!(actions.is_empty());
    }
}
