//! Submitting a stack layer by layer, with per-layer progress.

use crate::stack::Layer;
use crate::tui::action::Action;
use crate::tui::effects::Effects;
use crate::tui::state::submit_progress::SubmitProgress;

use super::App;

impl App {
    pub(super) fn reduce_submit(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::SubmitStack { stack_index } => {
                if let Some(progress) = &self.state.submit_progress
                    && !progress.finished
                {
                    self.state.status = Some("a submit is already in progress".to_string());
                    return Vec::new();
                }

                if self.submit_layers(*stack_index).is_none() {
                    self.state.status = Some("selected stack has no layers to submit".to_string());
                    return Vec::new();
                }

                vec![Action::SubmitStarted {
                    stack_index: *stack_index,
                }]
            }
            Action::SubmitStarted { stack_index } => {
                let Some(layers) = self.submit_layers(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };

                let branches = layers.iter().map(|layer| layer.branch.clone()).collect();
                self.state.submit_progress = Some(SubmitProgress::new(*stack_index, branches));
                effects.submit_stack(
                    *stack_index,
                    layers,
                    crate::config::get().default_remote.clone(),
                    self.state.submit_options,
                );
                Vec::new()
            }
            Action::SubmitLayerProgress {
                stack_index,
                layer_index,
                status,
            } => {
                if let Some(progress) = &mut self.state.submit_progress
                    && progress.stack_index == *stack_index
                {
                    progress.set_status(*layer_index, status.clone());
                }
                Vec::new()
            }
            Action::SubmitFinished { stack_index } => {
                if let Some(progress) = &mut self.state.submit_progress
                    && progress.stack_index == *stack_index
                {
                    progress.finished = true;
                    return vec![Action::RefreshStacks];
                }
                Vec::new()
            }
            Action::ToggleSubmitAuto => {
                self.state.submit_options.auto = !self.state.submit_options.auto;
                Vec::new()
            }
            Action::ToggleSubmitOpen => {
                self.state.submit_options.open = !self.state.submit_options.open;
                Vec::new()
            }
            Action::DismissSubmit => {
                self.state.submit_progress = None;
                Vec::new()
            }
            _ => unreachable!("routed to reduce_submit by App::apply_action"),
        }
    }

    /// The layers to submit for `stack_index`, or `None` when the stack has
    /// none (already vanished, or has no layers to push).
    fn submit_layers(&self, stack_index: usize) -> Option<Vec<Layer>> {
        let stack = self.state.stacks.get(stack_index)?;
        if stack.layers.is_empty() {
            return None;
        }
        Some(stack.layers.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    /// Submits every layer of `stack` through the real reducer and effect
    /// runner, returning each layer's final status.
    async fn submit_through_effects(
        stack: StackSummary,
        shell: &MockShell,
    ) -> Vec<LayerSubmitStatus> {
        let mut app = App::new();
        app.state.stacks = vec![stack];

        for action in app
            .settle(&Action::SubmitStarted { stack_index: 0 }, shell)
            .await
        {
            app.apply(&action);
        }

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert!(progress.finished);
        progress
            .layers
            .iter()
            .map(|layer| layer.status.clone())
            .collect()
    }

    fn push_ok(
        branch: &str,
    ) -> (
        &'static str,
        Vec<String>,
        Result<crate::shell::ShellOutput, ShellError>,
    ) {
        (
            "git",
            vec![
                "push".to_string(),
                "--force-with-lease".to_string(),
                "--set-upstream".to_string(),
                "origin".to_string(),
                format!("{branch}:{branch}"),
            ],
            Ok(crate::shell::ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }),
        )
    }

    fn mock_when(
        shell: MockShell,
        call: (
            &'static str,
            Vec<String>,
            Result<crate::shell::ShellOutput, ShellError>,
        ),
    ) -> MockShell {
        let (program, args, result) = call;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        shell.when(program, &args, result)
    }

    #[tokio::test]
    async fn submit_stack_action_starts_tracking_progress_for_every_layer() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app
            .settle(&Action::SubmitStack { stack_index: 0 }, &shell)
            .await;
        assert!(matches!(
            follow_ups.as_slice(),
            [Action::SubmitStarted { stack_index: 0 }]
        ));

        app.apply(&follow_ups[0]);

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert_eq!(progress.total(), 2);
        assert_eq!(progress.completed_count(), 0);
        assert!(!progress.finished);
    }

    #[tokio::test]
    async fn submit_stack_action_refuses_to_start_a_second_run_while_one_is_in_progress() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        app.state.submit_progress =
            Some(SubmitProgress::new(0, vec!["stack-a-layer-0".to_string()]));

        let follow_ups = app
            .settle(&Action::SubmitStack { stack_index: 0 }, &shell)
            .await;

        assert!(follow_ups.is_empty());
        assert!(app.state.status.is_some());
    }

    #[tokio::test]
    async fn submit_layer_progress_action_updates_only_the_matching_layer() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(
            0,
            vec!["a".to_string(), "b".to_string()],
        ));

        app.settle(
            &Action::SubmitLayerProgress {
                stack_index: 0,
                layer_index: 1,
                status: LayerSubmitStatus::Failed("boom".to_string()),
            },
            &shell,
        )
        .await;

        let progress = app.state.submit_progress.as_ref().unwrap();
        assert_eq!(progress.layers[0].status, LayerSubmitStatus::Pending);
        assert_eq!(
            progress.layers[1].status,
            LayerSubmitStatus::Failed("boom".to_string())
        );
    }

    #[tokio::test]
    async fn submit_finished_action_marks_progress_finished_and_refreshes_stacks() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        let follow_ups = app
            .settle(&Action::SubmitFinished { stack_index: 0 }, &shell)
            .await;

        assert!(app.state.submit_progress.as_ref().unwrap().finished);
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn toggle_actions_flip_submit_options() {
        let shell = MockShell::new();
        let mut app = App::new();
        assert!(!app.state.submit_options.auto);
        assert!(!app.state.submit_options.open);

        app.settle(&Action::ToggleSubmitAuto, &shell).await;
        app.settle(&Action::ToggleSubmitOpen, &shell).await;

        assert!(app.state.submit_options.auto);
        assert!(app.state.submit_options.open);
    }

    #[tokio::test]
    async fn dismiss_submit_clears_progress() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));

        app.settle(&Action::DismissSubmit, &shell).await;

        assert!(app.state.submit_progress.is_none());
    }

    /// Reproduces the issue's acceptance criterion directly: layer 2 of 4
    /// fails to push while the other three succeed, and the failure must be
    /// visible on that layer alone rather than failing the whole submit.
    #[tokio::test]
    async fn submit_through_effects_reports_a_partial_failure_per_layer() {
        let stack = stack_summary("stack-a", 4);

        let mut shell = MockShell::new();
        shell = mock_when(shell, push_ok("stack-a-layer-0"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-0",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/1".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        shell = shell.when(
            "git",
            &[
                "push",
                "--force-with-lease",
                "--set-upstream",
                "origin",
                "stack-a-layer-1:stack-a-layer-1",
            ],
            Err(ShellError::CommandFailed {
                program: "git".to_string(),
                output: crate::shell::ShellOutput {
                    stdout: String::new(),
                    stderr: "stale info".to_string(),
                    exit_code: 1,
                },
            }),
        );
        shell = mock_when(shell, push_ok("stack-a-layer-2"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-2",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/3".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        shell = mock_when(shell, push_ok("stack-a-layer-3"));
        shell = shell.when(
            "gh",
            &[
                "pr",
                "create",
                "--head",
                "stack-a-layer-3",
                "--base",
                "main",
                "--fill",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "https://github.com/o/r/pull/4".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let statuses = submit_through_effects(stack, &shell).await;

        assert_eq!(statuses.len(), 4);
        assert_eq!(
            statuses[0],
            LayerSubmitStatus::PullRequestCreated { number: 1 }
        );
        assert!(matches!(&statuses[1], LayerSubmitStatus::Failed(_)));
        assert_eq!(
            statuses[2],
            LayerSubmitStatus::PullRequestCreated { number: 3 }
        );
        assert_eq!(
            statuses[3],
            LayerSubmitStatus::PullRequestCreated { number: 4 }
        );
    }

    #[tokio::test]
    async fn submit_through_effects_reports_every_layer_succeeding() {
        let stack = stack_summary("stack-a", 2);

        let mut shell = MockShell::new();
        for (index, branch) in ["stack-a-layer-0", "stack-a-layer-1"].iter().enumerate() {
            shell = mock_when(shell, push_ok(branch));
            shell = shell.when(
                "gh",
                &["pr", "create", "--head", branch, "--base", "main", "--fill"],
                Ok(crate::shell::ShellOutput {
                    stdout: format!("https://github.com/o/r/pull/{}", index + 1),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            );
        }

        let statuses = submit_through_effects(stack, &shell).await;

        assert_eq!(
            statuses,
            vec![
                LayerSubmitStatus::PullRequestCreated { number: 1 },
                LayerSubmitStatus::PullRequestCreated { number: 2 },
            ]
        );
    }
}
