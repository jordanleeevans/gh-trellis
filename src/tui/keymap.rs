use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyIntent {
    MoveDown,
    MoveUp,
    FocusNext,
    FocusPrevious,
    DrillIn,
    Back,
    Refresh,
    Checkout,
    ToggleDiff,
    Submit,
    ToggleSubmitAuto,
    ToggleSubmitOpen,
    Help,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    End,
    OpenExternal,
    DismissMessage,
}

pub(crate) fn key_intent(key: KeyEvent) -> Option<KeyIntent> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(KeyIntent::HalfPageDown),
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Some(KeyIntent::HalfPageUp),
        (KeyCode::Char('G'), _) => Some(KeyIntent::End),
        (KeyCode::Down | KeyCode::Char('j'), _) => Some(KeyIntent::MoveDown),
        (KeyCode::Up | KeyCode::Char('k'), _) => Some(KeyIntent::MoveUp),
        (KeyCode::Tab, _) => Some(KeyIntent::FocusNext),
        (KeyCode::BackTab, _) => Some(KeyIntent::FocusPrevious),
        (KeyCode::Enter | KeyCode::Char(' '), _) => Some(KeyIntent::DrillIn),
        (KeyCode::Esc | KeyCode::Char('q'), _) => Some(KeyIntent::Back),
        (KeyCode::Char('r'), _) => Some(KeyIntent::Refresh),
        (KeyCode::Char('c'), _) => Some(KeyIntent::Checkout),
        (KeyCode::Char('d'), _) => Some(KeyIntent::ToggleDiff),
        (KeyCode::Char('s'), _) => Some(KeyIntent::Submit),
        (KeyCode::Char('a'), _) => Some(KeyIntent::ToggleSubmitAuto),
        (KeyCode::Char('p'), _) => Some(KeyIntent::ToggleSubmitOpen),
        (KeyCode::Char('?'), _) => Some(KeyIntent::Help),
        (KeyCode::PageDown, _) => Some(KeyIntent::PageDown),
        (KeyCode::PageUp | KeyCode::Backspace, _) => Some(KeyIntent::PageUp),
        (KeyCode::Char('o') | KeyCode::Char('O'), _) => Some(KeyIntent::OpenExternal),
        (KeyCode::Char('x'), _) => Some(KeyIntent::DismissMessage),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_global_navigation_keys() {
        assert_eq!(
            key_intent(key(KeyCode::Char('j'))),
            Some(KeyIntent::MoveDown)
        );
        assert_eq!(key_intent(key(KeyCode::Down)), Some(KeyIntent::MoveDown));
        assert_eq!(key_intent(key(KeyCode::Char('k'))), Some(KeyIntent::MoveUp));
        assert_eq!(key_intent(key(KeyCode::Up)), Some(KeyIntent::MoveUp));
        assert_eq!(key_intent(key(KeyCode::Tab)), Some(KeyIntent::FocusNext));
        assert_eq!(
            key_intent(key(KeyCode::BackTab)),
            Some(KeyIntent::FocusPrevious)
        );
        assert_eq!(key_intent(key(KeyCode::Enter)), Some(KeyIntent::DrillIn));
        assert_eq!(
            key_intent(key(KeyCode::Char(' '))),
            Some(KeyIntent::DrillIn)
        );
        assert_eq!(key_intent(key(KeyCode::Esc)), Some(KeyIntent::Back));
        assert_eq!(key_intent(key(KeyCode::Char('q'))), Some(KeyIntent::Back));
        assert_eq!(
            key_intent(key(KeyCode::Char('c'))),
            Some(KeyIntent::Checkout)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('d'))),
            Some(KeyIntent::ToggleDiff)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('o'))),
            Some(KeyIntent::OpenExternal)
        );
        assert_eq!(
            key_intent(ctrl_key(KeyCode::Char('d'))),
            Some(KeyIntent::HalfPageDown)
        );
        assert_eq!(
            key_intent(ctrl_key(KeyCode::Char('u'))),
            Some(KeyIntent::HalfPageUp)
        );
        assert_eq!(key_intent(key(KeyCode::Char('G'))), Some(KeyIntent::End));
        assert_eq!(key_intent(key(KeyCode::Char('?'))), Some(KeyIntent::Help));
        assert_eq!(
            key_intent(key(KeyCode::Char('x'))),
            Some(KeyIntent::DismissMessage)
        );
        assert_eq!(key_intent(key(KeyCode::Char('s'))), Some(KeyIntent::Submit));
        assert_eq!(
            key_intent(key(KeyCode::Char('a'))),
            Some(KeyIntent::ToggleSubmitAuto)
        );
        assert_eq!(
            key_intent(key(KeyCode::Char('p'))),
            Some(KeyIntent::ToggleSubmitOpen)
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }
}
