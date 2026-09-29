//! `gh stack sync`: confirmation, running it, and reporting the outcome.

use crate::stack::SyncOutcome;
use crate::tui::action::Action;
use crate::tui::components::sync_confirm::sync_modal;
use crate::tui::effects::Effects;

use super::App;

impl App {
    pub(super) fn reduce_sync(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::SyncStack { stack_index } => {
                if self.state.sync_in_flight {
                    return vec![Action::SetError(
                        "a sync is already in progress".to_string(),
                    )];
                }
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    return vec![Action::SetError(
                        "selected stack is no longer available".to_string(),
                    )];
                };
                if stack.layers.is_empty() {
                    return vec![Action::SetError(
                        "selected stack has no layers to sync".to_string(),
                    )];
                }
                // `gh stack sync` always acts on the checked-out stack.
                if !stack.is_current {
                    return vec![Action::SetError(
                        "sync only works on the checked-out stack; check it out first (c)"
                            .to_string(),
                    )];
                }

                vec![Action::ShowConfirm(sync_modal(
                    *stack_index,
                    stack,
                    &crate::config::get().default_remote,
                    self.state.sync_options,
                ))]
            }
            Action::SyncStarted { stack_index } => {
                if self.state.stacks.get(*stack_index).is_none() {
                    return vec![Action::SetError(
                        "selected stack is no longer available".to_string(),
                    )];
                }
                self.state.sync_in_flight = true;
                effects.sync_stack(
                    crate::config::get().default_remote.clone(),
                    self.state.sync_options,
                );
                Vec::new()
            }
            Action::SyncFinished { result } => {
                self.state.sync_in_flight = false;
                match result {
                    Err(message) => vec![Action::SetError(message.clone())],
                    Ok(SyncOutcome::Conflict) => vec![
                        Action::SetError(format!(
                            "sync hit a rebase conflict; your branches were restored. Rebase the stack ({}) to resolve it, then sync again",
                            crate::tui::keymap::current()
                                .short_label(crate::tui::keymap::KeyIntent::RebaseStack)
                                .unwrap_or_else(|| "unbound".to_string())
                        )),
                        Action::RefreshStacks,
                    ],
                    Ok(SyncOutcome::Diverged) => vec![
                        Action::SetError(
                            "local and remote stacks have diverged; sync stopped without pushing. Run `gh stack sync` in a terminal to resolve it"
                                .to_string(),
                        ),
                        Action::RefreshStacks,
                    ],
                    Ok(outcome) => {
                        self.state.status = Some(
                            match outcome {
                                SyncOutcome::Synced => "stack synced",
                                SyncOutcome::BranchesSynced => {
                                    "branches rebased and pushed (no remote stack updated)"
                                }
                                _ => "sync finished",
                            }
                            .to_string(),
                        );
                        vec![Action::RefreshStacks]
                    }
                }
            }
            Action::ToggleSyncPrune => {
                self.state.sync_options.prune = !self.state.sync_options.prune;
                Vec::new()
            }
            _ => unreachable!("routed to reduce_sync by App::apply_action"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    fn sync_ok(stderr: &str) -> Result<crate::shell::ShellOutput, ShellError> {
        Ok(crate::shell::ShellOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code: 0,
        })
    }

    fn current_stack(label: &str, layers: usize) -> StackSummary {
        StackSummary {
            is_current: true,
            ..stack_summary(label, layers)
        }
    }

    #[tokio::test]
    async fn sync_stack_action_asks_for_confirmation_naming_the_branches() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 2)];

        let follow_ups = app
            .settle(&Action::SyncStack { stack_index: 0 }, &shell)
            .await;

        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a confirm modal, got {follow_ups:?}");
        };
        assert!(!modal.danger);
        assert!(modal.body.contains("stack-a-layer-0"));
        assert!(modal.body.contains("stack-a-layer-1"));
        assert!(
            shell.calls().is_empty(),
            "nothing may run before confirming"
        );
    }

    #[tokio::test]
    async fn sync_stack_action_refuses_a_stack_that_is_not_checked_out() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(&Action::SyncStack { stack_index: 0 }, &shell)
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn confirming_the_sync_modal_starts_the_sync() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 1)];
        let follow_ups = app
            .settle(&Action::SyncStack { stack_index: 0 }, &shell)
            .await;
        app.apply(&follow_ups[0]);

        let accepted = app.apply(&Action::ConfirmAccept);

        assert!(matches!(
            accepted.as_slice(),
            [Action::SyncStarted { stack_index: 0 }]
        ));
    }

    #[tokio::test]
    async fn sync_started_runs_gh_stack_sync_and_reports_the_outcome() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "sync", "--remote", "origin"],
            sync_ok("Stack synced"),
        );
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 1)];

        let results = app
            .settle(&Action::SyncStarted { stack_index: 0 }, &shell)
            .await;

        assert!(app.state.sync_in_flight);
        let [
            Action::SyncFinished {
                result: Ok(SyncOutcome::Synced),
            },
        ] = results.as_slice()
        else {
            panic!("unexpected results: {results:?}");
        };

        let follow_ups = app.apply(&results[0]);
        assert!(!app.state.sync_in_flight);
        assert_eq!(app.state.status.as_deref(), Some("stack synced"));
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn sync_with_prune_enabled_passes_prune() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "sync", "--remote", "origin", "--prune"],
            sync_ok("Branches synced"),
        );
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 1)];
        app.apply(&Action::ToggleSyncPrune);
        assert!(app.state.sync_options.prune);

        let results = app
            .settle(&Action::SyncStarted { stack_index: 0 }, &shell)
            .await;

        assert!(matches!(
            results.as_slice(),
            [Action::SyncFinished {
                result: Ok(SyncOutcome::BranchesSynced),
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn sync_failure_is_reported_through_friendly_errors() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "sync", "--remote", "origin"],
            Err(ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "push rejected".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 1)];

        let results = app
            .settle(&Action::SyncStarted { stack_index: 0 }, &shell)
            .await;
        let follow_ups = app.apply(&results[0]);

        assert!(!app.state.sync_in_flight);
        let [Action::SetError(message)] = follow_ups.as_slice() else {
            panic!("expected an error, got {follow_ups:?}");
        };
        assert_eq!(message, "sync stack: push rejected");
    }

    #[test]
    fn sync_conflict_and_divergence_surface_as_errors_and_still_refresh() {
        for outcome in [SyncOutcome::Conflict, SyncOutcome::Diverged] {
            let mut app = App::new();
            app.state.sync_in_flight = true;

            let follow_ups = app.apply(&Action::SyncFinished {
                result: Ok(outcome),
            });

            assert!(!app.state.sync_in_flight);
            assert!(app.state.status.is_none());
            assert!(matches!(
                follow_ups.as_slice(),
                [Action::SetError(_), Action::RefreshStacks]
            ));
        }
    }

    #[test]
    fn a_second_sync_is_refused_while_one_is_running() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack("stack-a", 1)];
        app.state.sync_in_flight = true;

        let follow_ups = app.apply(&Action::SyncStack { stack_index: 0 });

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }
}
