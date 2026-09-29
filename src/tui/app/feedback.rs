//! Errors, status and the shared confirm modal.

use crate::tui::action::Action;

use super::App;

impl App {
    pub(super) fn reduce_feedback(&mut self, action: &Action) -> Vec<Action> {
        match action {
            Action::ClearStatus => {
                self.state.status = None;
                Vec::new()
            }
            Action::SetError(message) => {
                self.state.error = Some(message.clone());
                Vec::new()
            }
            Action::ClearError => {
                self.state.error = None;
                Vec::new()
            }
            Action::ShowConfirm(modal) => {
                self.state.confirm = Some(modal.clone());
                Vec::new()
            }
            Action::ConfirmCancel => {
                self.state.confirm = None;
                Vec::new()
            }
            Action::ConfirmAccept => match self.state.confirm.take() {
                Some(modal) if modal.can_confirm() => {
                    modal.into_confirmed_action().into_iter().collect()
                }
                Some(modal) => {
                    // Danger modal, phrase not typed correctly yet: keep it
                    // open rather than firing or dismissing it.
                    self.state.confirm = Some(modal);
                    Vec::new()
                }
                None => Vec::new(),
            },
            Action::ConfirmInput(c) => {
                if let Some(modal) = self.state.confirm.as_mut() {
                    modal.push_char(*c);
                }
                Vec::new()
            }
            Action::ConfirmBackspace => {
                if let Some(modal) = self.state.confirm.as_mut() {
                    modal.pop_char();
                }
                Vec::new()
            }
            _ => unreachable!("routed to reduce_feedback by App::apply_action"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    #[tokio::test]
    async fn quit_key_shows_confirmation_instead_of_quitting_immediately() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let actions = app.handle_key(key(KeyCode::Char('q')));
        assert!(matches!(actions.as_slice(), [Action::ShowConfirm(_)]));

        app.dispatch_now(actions);

        let modal = app.state.confirm.as_ref().expect("modal should be shown");
        assert!(!modal.danger);
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn confirming_quit_modal_actually_quits() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_now(show);
        assert!(app.state.confirm.is_some());
        assert!(!app.state.should_quit);

        let accept = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(accept.as_slice(), [Action::ConfirmAccept]));
        app.dispatch_now(accept);

        assert!(app.state.should_quit);
        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn cancelling_quit_modal_leaves_app_running() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let show = app.handle_key(key(KeyCode::Char('q')));
        app.dispatch_now(show);

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_now(cancel);

        assert!(app.state.confirm.is_none());
        assert!(!app.state.should_quit);
    }

    #[tokio::test]
    async fn danger_modal_requires_typed_phrase_before_enter_fires_action() {
        let mut app = App::new();

        app.state.confirm = Some(
            ConfirmModal::new(
                "Merge PR?",
                "This merges the stack into main.",
                "Merge",
                true,
            )
            .on_confirm(Action::ClearStatus),
        );

        // Enter does nothing yet: no phrase has been typed at all.
        let premature = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(premature);
        assert!(app.state.confirm.is_some());

        // Typing the wrong phrase also keeps it open.
        for c in "no".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }
        let premature = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(premature);
        assert!(app.state.confirm.is_some());

        for _ in 0.."no".len() {
            let backspace = app.handle_key(key(KeyCode::Backspace));
            app.dispatch_now(backspace);
        }

        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }
        assert_eq!(app.state.confirm.as_ref().unwrap().typed_input(), "yes");

        let accept = app.handle_key(key(KeyCode::Enter));
        app.dispatch_now(accept);

        assert!(app.state.confirm.is_none());
    }

    #[tokio::test]
    async fn any_key_clears_the_status_message() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.status = Some("stack synced".to_string());

        let actions = app.handle_key(key(KeyCode::Char('j')));
        app.dispatch_now(actions);

        assert!(app.state.status.is_none());
    }

    #[tokio::test]
    async fn a_status_set_by_the_same_key_press_survives() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.status = Some("old message".to_string());

        // The layer has no pull request, so `o` sets a new status.
        app.dispatch_now(vec![Action::ShowLayers(0)]);
        let actions = app.handle_key(key(KeyCode::Char('o')));
        app.dispatch_now(actions);

        assert_eq!(
            app.state.status.as_deref(),
            Some("selected layer has no pull request")
        );
    }

    #[tokio::test]
    async fn danger_modal_esc_cancels_without_firing_action() {
        let mut app = App::new();
        app.state.status = Some("untouched".to_string());

        app.state.confirm = Some(
            ConfirmModal::new(
                "Merge PR?",
                "This merges the stack into main.",
                "Merge",
                true,
            )
            .on_confirm(Action::ClearStatus),
        );

        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(matches!(cancel.as_slice(), [Action::ConfirmCancel]));
        app.dispatch_now(cancel);

        assert!(app.state.confirm.is_none());
        // The attached action (ClearStatus) must never have fired.
        assert_eq!(app.state.status.as_deref(), Some("untouched"));
    }
}
