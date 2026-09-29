//! Stack list refresh: request ids, loading state, and applying a fresh stack list.

use crate::stack::StackSummary;
use crate::tui::action::Action;
use crate::tui::effects::Effects;
use crate::tui::messages::friendly_stack_refresh_error;
use crate::tui::state::Screen;

use super::App;

impl App {
    pub(super) fn reduce_refresh(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::RefreshStacks => {
                self.state.refresh_request_id += 1;
                vec![Action::StackRefreshStarted {
                    request_id: self.state.refresh_request_id,
                }]
            }
            Action::StackRefreshStarted { request_id } => {
                self.state.refresh_in_flight = true;
                self.state.refresh_spinner_frame = 0;
                self.state.refresh_active_request_id = Some(*request_id);
                if !self.state.stacks.is_empty() {
                    self.state.last_successful_stacks = self.state.stacks.clone();
                }
                effects.load_stacks(*request_id);
                Vec::new()
            }
            Action::StackRefreshSucceeded { request_id, result } => {
                if Some(*request_id) != self.state.refresh_active_request_id {
                    return Vec::new();
                }

                self.state.refresh_in_flight = false;
                self.state.refresh_active_request_id = None;

                match result {
                    Ok(stacks) => self.apply_stacks_loaded(stacks.clone()),
                    Err(error) => vec![Action::SetError(format!(
                        "failed to refresh stacks: {}",
                        friendly_stack_refresh_error(error)
                    ))],
                }
            }
            Action::Tick => {
                if self.state.refresh_in_flight || self.state.merge_in_flight {
                    self.state.refresh_spinner_frame =
                        self.state.refresh_spinner_frame.wrapping_add(1);
                }
                Vec::new()
            }
            Action::StacksLoaded(_) => {
                if let Screen::Layers(index) = self.state.screen
                    && index >= self.state.stacks.len()
                {
                    self.state.screen = Screen::List;
                    self.state.status = Some("selected stack is no longer available".to_string());
                }
                Vec::new()
            }
            _ => unreachable!("routed to reduce_refresh by App::apply_action"),
        }
    }

    fn selected_stack_label(&self) -> Option<String> {
        match self.state.screen {
            Screen::Layers(index) => self
                .state
                .stacks
                .get(index)
                .map(|stack| stack.label.clone()),
            Screen::List => None,
        }
    }

    fn apply_stacks_loaded(&mut self, stacks: Vec<StackSummary>) -> Vec<Action> {
        let selected_label = self.selected_stack_label();
        let selected_index = selected_label
            .and_then(|label| stacks.iter().position(|stack| stack.label == label))
            .or_else(|| stacks.iter().position(|stack| stack.is_current))
            .or(if stacks.is_empty() { None } else { Some(0) });

        self.state.stacks = stacks;
        self.state.last_successful_stacks = self.state.stacks.clone();
        self.state.screen = match selected_index {
            Some(index) => Screen::Layers(index),
            None => Screen::List,
        };

        vec![Action::StacksLoaded(selected_index)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    #[tokio::test]
    async fn refresh_stacks_creates_started_action_without_blocking() {
        let shell = MockShell::new();
        let mut app = App::new();

        let follow_ups = app.settle(&Action::RefreshStacks, &shell).await;

        assert!(matches!(
            follow_ups.as_slice(),
            [Action::StackRefreshStarted { request_id: 1 }]
        ));
        assert!(!app.state.refresh_in_flight);
    }

    #[tokio::test]
    async fn refresh_started_sets_loading_and_keeps_existing_stacks() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        app.apply(&Action::StackRefreshStarted { request_id: 9 });

        assert!(app.state.refresh_in_flight);
        assert_eq!(app.state.refresh_active_request_id, Some(9));
        assert_eq!(app.state.stacks.len(), 1);
        assert_eq!(app.state.last_successful_stacks.len(), 1);
    }

    #[tokio::test]
    async fn refresh_success_ignores_outdated_request_ids() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.refresh_active_request_id = Some(2);

        app.settle(
            &Action::StackRefreshSucceeded {
                request_id: 1,
                result: Ok(vec![stack_summary("stack-b", 1)]),
            },
            &shell,
        )
        .await;

        assert_eq!(app.state.stacks[0].label, "stack-a");
    }

    #[tokio::test]
    async fn refresh_failure_sets_dismissible_error() {
        let mut app = App::new();
        app.state.refresh_active_request_id = Some(4);
        app.state.refresh_in_flight = true;

        app.dispatch_now(vec![Action::StackRefreshSucceeded {
            request_id: 4,
            result: Err("network timeout".to_string()),
        }]);

        assert!(!app.state.refresh_in_flight);
        assert!(app.state.error.is_some());
    }

    #[test]
    fn a_background_refresh_does_not_wipe_the_status_message() {
        let mut app = App::new();
        app.state.status = Some("stack synced".to_string());
        app.state.refresh_active_request_id = Some(1);

        app.dispatch_now(vec![Action::StackRefreshSucceeded {
            request_id: 1,
            result: Ok(vec![stack_summary("stack-a", 1)]),
        }]);

        assert_eq!(app.state.status.as_deref(), Some("stack synced"));
    }

    #[tokio::test]
    async fn tick_advances_spinner_while_refreshing() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.refresh_in_flight = true;

        app.settle(&Action::Tick, &shell).await;
        assert_eq!(app.state.refresh_spinner_frame, 1);
    }
}
