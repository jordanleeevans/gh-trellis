//! Single-command stack operations: checkout, add layer, open PR, unstack.

use crate::stack::{StackSummary, UnstackScope};
use crate::tui::action::Action;
use crate::tui::components::confirm::ConfirmModal;
use crate::tui::effects::Effects;

use super::App;

impl App {
    pub(super) fn reduce_stack_ops(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::CheckoutSelected {
                stack_index,
                layer_index,
            } => {
                if let Some(branch) = self.checkout_target(*stack_index, *layer_index) {
                    effects.checkout(branch);
                }
                Vec::new()
            }
            Action::AddLayer {
                stack_index,
                branch,
                message,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                // `gh stack add` operates on whatever stack is currently
                // checked out; refuse rather than silently mutating a
                // different stack than the one the user was browsing.
                if !stack.is_current {
                    return vec![Action::SetError(
                        "checkout this stack before adding a layer".to_string(),
                    )];
                }

                effects.add_layer(branch.clone(), message.clone());
                Vec::new()
            }
            Action::UnstackSelected { stack_index, scope } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                // `gh stack unstack` with no argument acts on the stack of the
                // checked-out branch; refuse rather than unstacking a
                // different stack than the one selected.
                if !stack.is_current {
                    return vec![Action::SetError(
                        "checkout this stack before unstacking it".to_string(),
                    )];
                }

                let modal = unstack_modal(stack, *stack_index, *scope);
                vec![Action::ShowConfirm(modal)]
            }
            Action::RunUnstack { stack_index, scope } => {
                match self.state.stacks.get(*stack_index) {
                    Some(stack) if stack.is_current => effects.unstack(*scope),
                    Some(_) => {
                        return vec![Action::SetError(
                            "checkout this stack before unstacking it".to_string(),
                        )];
                    }
                    None => {
                        self.state.status =
                            Some("selected stack is no longer available".to_string());
                    }
                }
                Vec::new()
            }
            Action::OpenPullRequest {
                stack_index,
                layer_index,
            } => {
                match self
                    .state
                    .stacks
                    .get(*stack_index)
                    .and_then(|stack| stack.layers.get(*layer_index))
                    .and_then(|layer| layer.pull_request.as_ref())
                {
                    Some(pr) => effects.open_pull_request(pr.number),
                    None => {
                        self.state.status = Some("selected layer has no pull request".to_string())
                    }
                }
                Vec::new()
            }
            _ => unreachable!("routed to reduce_stack_ops by App::apply_action"),
        }
    }

    /// The branch `gh stack checkout` should target: the given layer, or
    /// for a whole stack its current layer (falling back to the top one).
    fn checkout_target(
        &mut self,
        stack_index: usize,
        layer_index: Option<usize>,
    ) -> Option<String> {
        let Some(stack) = self.state.stacks.get(stack_index) else {
            self.state.status = Some("selected stack is no longer available".to_string());
            return None;
        };

        let layer = match layer_index {
            Some(index) => stack.layers.get(index).or_else(|| {
                self.state.status = Some("selected layer is no longer available".to_string());
                None
            }),
            None => stack
                .layers
                .iter()
                .find(|layer| layer.is_current)
                .or_else(|| stack.layers.last())
                .or_else(|| {
                    self.state.status = Some("selected stack has no layers".to_string());
                    None
                }),
        }?;

        Some(layer.branch.clone())
    }
}

/// Builds the confirmation for unstacking `stack`.
///
/// The copy only states what was verified in `gh stack unstack`'s source
/// (see `stack::unstack`): `--local` edits local tracking and never contacts
/// GitHub; the remote variant dissolves the stack on GitHub, and the command
/// deletes no branches and closes no PRs.
fn unstack_modal(stack: &StackSummary, stack_index: usize, scope: UnstackScope) -> ConfirmModal {
    let on_confirm = Action::RunUnstack { stack_index, scope };
    match scope {
        UnstackScope::Local => ConfirmModal::new(
            "Remove stack from local tracking?",
            format!(
                "Stops tracking \"{}\" in gh-stack on this machine only. Nothing on \
                 GitHub is contacted or changed: remote branches, pull requests and \
                 the GitHub stack stay as they are, and local branches are kept.",
                stack.label
            ),
            "Remove locally",
            false,
        ),
        UnstackScope::LocalAndRemote => {
            let prs: Vec<String> = stack
                .layers
                .iter()
                .filter_map(|layer| layer.pull_request.as_ref())
                .map(|pr| format!("#{}", pr.number))
                .collect();
            let affected = if prs.is_empty() {
                "This stack has no pull requests yet.".to_string()
            } else {
                format!("Pull requests affected on GitHub: {}.", prs.join(", "))
            };
            let branches: Vec<&str> = stack.layers.iter().map(|l| l.branch.as_str()).collect();
            ConfirmModal::new(
                "Unstack on GitHub and locally?",
                format!(
                    "Dissolves \"{}\" on GitHub, unlinking its pull requests from one \
                     another, then removes local tracking. {affected} Branches: {}. \
                     This command does not delete branches or close pull requests, but \
                     it cannot be undone by trellis. PRs queued for merge or with \
                     auto-merge stay stacked.",
                    stack.label,
                    branches.join(", ")
                ),
                "Unstack on GitHub",
                true,
            )
        }
    }
    .on_confirm(on_confirm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    #[tokio::test]
    async fn checkout_selected_stack_uses_current_layer_when_present() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-0"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];
        app.state.stacks[0].layers[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_stack_uses_top_layer_when_none_current() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-1"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_layer_uses_selected_layer_branch() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-1"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: Some(1),
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn checkout_selected_returns_set_error_on_checkout_failure() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "checkout", "stack-a-layer-0"],
            Err(crate::shell::ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let follow_ups = app
            .settle(
                &Action::CheckoutSelected {
                    stack_index: 0,
                    layer_index: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    fn unstack_ok(args: &[&str]) -> MockShell {
        MockShell::new().when(
            "gh",
            args,
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        )
    }

    fn current_stack_with_pr() -> App {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];
        app.state.stacks[0].is_current = true;
        app.state.stacks[0].layers[0].pull_request = Some(crate::stack::PullRequestRef {
            number: 44,
            url: "https://example.test/44".to_string(),
            state: "OPEN".to_string(),
            title: None,
            is_draft: None,
            checks_status: None,
            review_decision: None,
        });
        app
    }

    #[tokio::test]
    async fn unstack_local_shows_non_danger_confirm_naming_local_only() {
        let mut app = current_stack_with_pr();

        let follow_ups = app.apply(&Action::UnstackSelected {
            stack_index: 0,
            scope: UnstackScope::Local,
        });

        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a confirm modal, got {follow_ups:?}");
        };
        assert!(!modal.danger);
        assert!(modal.can_confirm());
        assert!(modal.body.contains("local"));
        assert!(modal.body.contains("Nothing on GitHub"));
    }

    #[tokio::test]
    async fn unstack_remote_shows_danger_confirm_naming_prs() {
        let mut app = current_stack_with_pr();

        let follow_ups = app.apply(&Action::UnstackSelected {
            stack_index: 0,
            scope: UnstackScope::LocalAndRemote,
        });

        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a confirm modal, got {follow_ups:?}");
        };
        assert!(modal.danger);
        assert!(!modal.can_confirm());
        assert!(modal.body.contains("#44"));
        assert!(modal.body.contains("stack-a-layer-0"));
    }

    #[tokio::test]
    async fn unstack_refuses_a_stack_that_is_not_checked_out() {
        let mut app = current_stack_with_pr();
        app.state.stacks[0].is_current = false;

        let follow_ups = app.apply(&Action::UnstackSelected {
            stack_index: 0,
            scope: UnstackScope::Local,
        });
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));

        let shell = MockShell::new();
        let follow_ups = app
            .settle(
                &Action::RunUnstack {
                    stack_index: 0,
                    scope: UnstackScope::Local,
                },
                &shell,
            )
            .await;
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
        assert!(shell.calls().is_empty());
    }

    #[tokio::test]
    async fn confirmed_local_unstack_runs_local_command_and_refreshes() {
        let shell = unstack_ok(&["stack", "unstack", "--local"]);
        let mut app = current_stack_with_pr();

        let follow_ups = app
            .settle(
                &Action::RunUnstack {
                    stack_index: 0,
                    scope: UnstackScope::Local,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
        assert_eq!(shell.calls().len(), 1);
    }

    #[tokio::test]
    async fn confirmed_remote_unstack_runs_remote_command_and_refreshes() {
        let shell = unstack_ok(&["stack", "unstack"]);
        let mut app = current_stack_with_pr();

        let follow_ups = app
            .settle(
                &Action::RunUnstack {
                    stack_index: 0,
                    scope: UnstackScope::LocalAndRemote,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn unstack_failure_is_reported_via_set_error() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "unstack"],
            Err(crate::shell::ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = current_stack_with_pr();

        let follow_ups = app
            .settle(
                &Action::RunUnstack {
                    stack_index: 0,
                    scope: UnstackScope::LocalAndRemote,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn danger_unstack_modal_only_fires_after_typing_the_phrase() {
        let mut app = current_stack_with_pr();
        let shown = app.apply(&Action::UnstackSelected {
            stack_index: 0,
            scope: UnstackScope::LocalAndRemote,
        });
        app.apply(&shown[0]);

        assert!(app.apply(&Action::ConfirmAccept).is_empty());
        for c in "yes".chars() {
            app.apply(&Action::ConfirmInput(c));
        }
        let confirmed = app.apply(&Action::ConfirmAccept);
        assert!(matches!(
            confirmed.as_slice(),
            [Action::RunUnstack {
                stack_index: 0,
                scope: UnstackScope::LocalAndRemote
            }]
        ));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_with_message() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer", "-m", "add new layer"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: Some("add new layer".to_string()),
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_runs_gh_stack_add_without_message_flag() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer"],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn add_layer_returns_set_error_on_failure() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "add", "feature/new-layer"],
            Err(crate::shell::ShellError::CommandFailed {
                program: "gh".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.stacks[0].is_current = true;

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn add_layer_refuses_to_run_on_a_non_current_stack() {
        // No responses registered: the shell must not be invoked at all.
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];

        let follow_ups = app
            .settle(
                &Action::AddLayer {
                    stack_index: 0,
                    branch: "feature/new-layer".to_string(),
                    message: None,
                },
                &shell,
            )
            .await;

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }
}
