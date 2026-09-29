//! `gh stack merge`: refusal checks, the danger confirmation, running it
//! and reporting the outcome. What the command does, and why trellis passes
//! `--yes` only after its own confirmation, is documented in
//! `stack::merge`.

use crate::stack::{MergeOutcome, MergeRefusal, StackSummary, merge_plan};
use crate::tui::action::Action;
use crate::tui::components::merge_confirm::merge_modal;
use crate::tui::effects::Effects;
use crate::tui::keymap::{self, KeyIntent};
use crate::tui::messages::merge_refusal_message;

use super::App;

impl App {
    pub(super) fn reduce_merge(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::MergeStack { stack_index } => {
                let stack = match self.mergeable_stack(*stack_index) {
                    Ok(stack) => stack,
                    Err(message) => return vec![Action::SetError(message)],
                };
                match merge_plan(stack) {
                    Ok(plan) => vec![Action::ShowConfirm(merge_modal(
                        *stack_index,
                        &stack.label,
                        &plan,
                        self.state.merge_method,
                    ))],
                    Err(MergeRefusal::AlreadyMerged) => {
                        self.state.status =
                            Some(merge_refusal_message(&MergeRefusal::AlreadyMerged));
                        Vec::new()
                    }
                    Err(refusal) => vec![Action::SetError(merge_refusal_message(&refusal))],
                }
            }
            Action::CycleMergeMethod => {
                self.state.merge_method = self.state.merge_method.next();
                self.state.status =
                    Some(format!("merge method: {}", self.state.merge_method.name()));
                Vec::new()
            }
            Action::MergeStarted {
                stack_index,
                prs,
                method,
            } => {
                // State may have changed between showing the modal and
                // confirming it (a refresh that was already running can
                // land in between). Only merge what the user was shown.
                let stack = match self.mergeable_stack(*stack_index) {
                    Ok(stack) => stack,
                    Err(message) => {
                        return vec![Action::SetError(format!("nothing was merged: {message}"))];
                    }
                };
                match merge_plan(stack) {
                    Ok(plan) if plan.pr_numbers() == *prs => {}
                    _ => {
                        return vec![Action::SetError(
                            "nothing was merged: the stack changed after you confirmed. Review it and merge again"
                                .to_string(),
                        )];
                    }
                }
                self.state.merge_in_flight = true;
                effects.merge_stack(*method);
                Vec::new()
            }
            Action::MergeFinished { result } => {
                self.state.merge_in_flight = false;
                match result {
                    Ok(outcome) => {
                        self.state.status = Some(merge_status(outcome));
                        vec![Action::RefreshStacks]
                    }
                    Err(message) => vec![Action::SetError(message.clone()), Action::RefreshStacks],
                }
            }
            _ => unreachable!("routed to reduce_merge by App::apply_action"),
        }
    }

    /// The stack at `stack_index`, if a merge may be offered for it now.
    fn mergeable_stack(&self, stack_index: usize) -> Result<&StackSummary, String> {
        if self.state.merge_in_flight {
            return Err("a merge is already in progress".to_string());
        }
        if self.state.sync_in_flight
            || self
                .state
                .submit_progress
                .as_ref()
                .is_some_and(|progress| !progress.finished)
        {
            return Err("wait for the running submit or sync to finish before merging".to_string());
        }
        let Some(stack) = self.state.stacks.get(stack_index) else {
            return Err("selected stack is no longer available".to_string());
        };
        // `gh stack merge` with no argument merges the checked-out stack.
        if !stack.is_current {
            let checkout = keymap::current()
                .short_label(KeyIntent::Checkout)
                .map(|key| format!(" ({key})"))
                .unwrap_or_default();
            return Err(format!(
                "merge only works on the checked-out stack; check it out first{checkout}"
            ));
        }
        Ok(stack)
    }
}

/// The footer status for a successful merge, naming each PR gh-stack
/// reported.
fn merge_status(outcome: &MergeOutcome) -> String {
    let list = |prs: &[u64]| {
        prs.iter()
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sync = keymap::current()
        .short_label(KeyIntent::Sync)
        .map(|key| format!("; sync ({key}) to update local branches"))
        .unwrap_or_default();
    match outcome {
        MergeOutcome::Merged { prs, base, sha } => {
            let sha = sha
                .as_deref()
                .map(|sha| format!(" ({sha})"))
                .unwrap_or_default();
            format!("merged {} into {base}{sha}{sync}", list(prs))
        }
        MergeOutcome::Enqueued { prs, base } => format!(
            "added {} to the merge queue for {base}; they merge as the queue processes them",
            list(prs)
        ),
        MergeOutcome::AlreadyMerged => "stack was already fully merged".to_string(),
        MergeOutcome::Completed => "merge finished; check the pull requests on GitHub".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellOutput;
    use crate::stack::{MergeMethod, PullRequestRef};
    use crate::tui::app::test_support::*;
    use crate::tui::state::submit_progress::SubmitProgress;

    fn pr(number: u64, draft: bool) -> PullRequestRef {
        PullRequestRef {
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            state: "OPEN".to_string(),
            title: Some(format!("Layer {number}")),
            is_draft: Some(draft),
            checks_status: None,
            review_decision: None,
        }
    }

    /// A checked-out stack of three open PRs, #44-#46.
    fn current_stack() -> StackSummary {
        let mut stack = stack_summary("stack-a", 3);
        stack.is_current = true;
        for (index, layer) in stack.layers.iter_mut().enumerate() {
            layer.pull_request = Some(pr(44 + index as u64, false));
        }
        stack
    }

    const MERGE_ARGS: &[&str] = &["stack", "merge", "--yes", "--merge"];

    fn merged_output() -> Result<ShellOutput, ShellError> {
        Ok(ShellOutput {
            stdout: String::new(),
            stderr: "\u{2713} Merged #44, #45, #46 into main (abc1234)\n".to_string(),
            exit_code: 0,
        })
    }

    fn type_phrase_and_accept(app: &mut App) -> Vec<Action> {
        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }
        let accept = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(accept.as_slice(), [Action::ConfirmAccept]));
        app.apply(&accept[0])
    }

    #[tokio::test]
    async fn merge_shows_a_danger_modal_naming_the_prs_in_order_and_runs_nothing() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];
        app.state.merge_method = MergeMethod::Squash;

        let follow_ups = app
            .settle(&Action::MergeStack { stack_index: 0 }, &shell)
            .await;

        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a confirm modal, got {follow_ups:?}");
        };
        assert!(modal.danger);
        let positions: Vec<usize> = [
            "1. #44 Layer 44 (stack-a-layer-0)",
            "2. #45 Layer 45 (stack-a-layer-1)",
            "3. #46 Layer 46 (stack-a-layer-2)",
        ]
        .iter()
        .map(|line| modal.body.find(line).expect(line))
        .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(modal.body.contains("into main"));
        assert!(modal.body.contains("Method: squash (--squash)"));
        assert!(
            shell.calls().is_empty(),
            "nothing may run before confirming"
        );
    }

    #[tokio::test]
    async fn merge_runs_only_after_the_phrase_is_typed() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];
        let show = app.apply(&Action::MergeStack { stack_index: 0 });
        app.dispatch_now(show);

        // Enter alone keeps the modal open and dispatches nothing.
        let premature = app.handle_key(key(KeyCode::Enter));
        assert!(app.apply(&premature[0]).is_empty());
        assert!(app.state.confirm.is_some());
        assert!(!app.state.merge_in_flight);

        let confirmed = type_phrase_and_accept(&mut app);
        let [
            Action::MergeStarted {
                stack_index: 0,
                prs,
                method: MergeMethod::Merge,
            },
        ] = confirmed.as_slice()
        else {
            panic!("expected MergeStarted, got {confirmed:?}");
        };
        assert_eq!(prs, &[44, 45, 46]);

        let shell = MockShell::new().when("gh", MERGE_ARGS, merged_output());
        let results = app.settle(&confirmed[0], &shell).await;
        assert!(app.state.merge_in_flight);
        assert_eq!(
            shell.calls(),
            vec![(
                "gh".to_string(),
                MERGE_ARGS.iter().map(|a| a.to_string()).collect()
            )]
        );
        assert!(matches!(
            results.as_slice(),
            [Action::MergeFinished { result: Ok(_) }]
        ));
    }

    #[tokio::test]
    async fn cancelling_the_modal_never_merges() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];
        let show = app.apply(&Action::MergeStack { stack_index: 0 });
        app.dispatch_now(show);
        for c in "yes".chars() {
            let typed = app.handle_key(key(KeyCode::Char(c)));
            app.dispatch_now(typed);
        }

        let cancel = app.handle_key(key(KeyCode::Esc));
        assert!(app.apply(&cancel[0]).is_empty());

        assert!(app.state.confirm.is_none());
        assert!(!app.state.merge_in_flight);
    }

    #[tokio::test]
    async fn successful_merge_reports_each_pr_and_refreshes() {
        let mut app = App::new();
        app.state.merge_in_flight = true;

        let follow_ups = app.apply(&Action::MergeFinished {
            result: Ok(MergeOutcome::Merged {
                prs: vec![44, 45, 46],
                base: "main".to_string(),
                sha: Some("abc1234".to_string()),
            }),
        });

        assert!(!app.state.merge_in_flight);
        assert_eq!(
            app.state.status.as_deref(),
            Some("merged #44, #45, #46 into main (abc1234); sync (S) to update local branches")
        );
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[tokio::test]
    async fn failed_merge_is_an_error_and_still_refreshes() {
        let shell = MockShell::new().when(
            "gh",
            MERGE_ARGS,
            Err(ShellError::CommandFailed {
                program: "gh".to_string(),
                output: ShellOutput {
                    stdout: String::new(),
                    stderr: "\u{2717} merge failed: #45 has failing required checks\nStack merges are atomic, so nothing was merged.\n".to_string(),
                    exit_code: 1,
                },
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];

        let results = app
            .settle(
                &Action::MergeStarted {
                    stack_index: 0,
                    prs: vec![44, 45, 46],
                    method: MergeMethod::Merge,
                },
                &shell,
            )
            .await;
        let follow_ups = app.apply(&results[0]);

        assert!(!app.state.merge_in_flight);
        let [Action::SetError(message), Action::RefreshStacks] = follow_ups.as_slice() else {
            panic!("expected an error and a refresh, got {follow_ups:?}");
        };
        assert_eq!(
            message,
            "merge failed, nothing was merged (stack merges are atomic): #45 has failing required checks"
        );
        assert!(app.state.status.is_none());
    }

    #[tokio::test]
    async fn a_stack_that_is_not_checked_out_is_refused() {
        let shell = MockShell::new();
        let mut app = App::new();
        let mut stack = current_stack();
        stack.is_current = false;
        app.state.stacks = vec![stack];

        let follow_ups = app
            .settle(&Action::MergeStack { stack_index: 0 }, &shell)
            .await;

        let [Action::SetError(message)] = follow_ups.as_slice() else {
            panic!("expected a refusal, got {follow_ups:?}");
        };
        assert!(message.contains("checked-out stack"));
        assert!(shell.calls().is_empty());
    }

    #[tokio::test]
    async fn a_draft_anywhere_in_the_stack_is_refused_like_gh_stack_does() {
        let shell = MockShell::new();
        let mut app = App::new();
        let mut stack = current_stack();
        stack.layers[1].pull_request = Some(pr(45, true));
        app.state.stacks = vec![stack];

        let follow_ups = app
            .settle(&Action::MergeStack { stack_index: 0 }, &shell)
            .await;

        let [Action::SetError(message)] = follow_ups.as_slice() else {
            panic!("expected a refusal, got {follow_ups:?}");
        };
        assert!(message.contains("#45 (stack-a-layer-1) is a draft"));
        assert!(app.state.confirm.is_none());
    }

    #[test]
    fn a_fully_merged_stack_is_a_notice_not_a_modal() {
        let mut app = App::new();
        let mut stack = current_stack();
        for layer in &mut stack.layers {
            layer.pull_request.as_mut().unwrap().state = "MERGED".to_string();
        }
        app.state.stacks = vec![stack];

        assert!(app.apply(&Action::MergeStack { stack_index: 0 }).is_empty());
        assert_eq!(
            app.state.status.as_deref(),
            Some("this stack is already fully merged")
        );
    }

    #[test]
    fn merge_is_refused_while_another_write_is_running() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];

        app.state.merge_in_flight = true;
        assert!(matches!(
            app.apply(&Action::MergeStack { stack_index: 0 }).as_slice(),
            [Action::SetError(_)]
        ));

        app.state.merge_in_flight = false;
        app.state.sync_in_flight = true;
        assert!(matches!(
            app.apply(&Action::MergeStack { stack_index: 0 }).as_slice(),
            [Action::SetError(_)]
        ));

        app.state.sync_in_flight = false;
        app.state.submit_progress = Some(SubmitProgress::new(0, vec!["a".to_string()]));
        assert!(matches!(
            app.apply(&Action::MergeStack { stack_index: 0 }).as_slice(),
            [Action::SetError(_)]
        ));
    }

    #[tokio::test]
    async fn a_confirmed_merge_is_refused_if_the_stack_changed_since() {
        let shell = MockShell::new();
        let mut app = App::new();
        let mut stack = current_stack();
        // A refresh landed after the modal: #45 went back to draft.
        stack.layers[1].pull_request = Some(pr(45, true));
        app.state.stacks = vec![stack];

        let results = app
            .settle(
                &Action::MergeStarted {
                    stack_index: 0,
                    prs: vec![44, 45, 46],
                    method: MergeMethod::Merge,
                },
                &shell,
            )
            .await;

        assert!(
            matches!(results.as_slice(), [Action::SetError(m)] if m.starts_with("nothing was merged"))
        );
        assert!(!app.state.merge_in_flight);
        assert!(shell.calls().is_empty());
    }

    #[tokio::test]
    async fn a_confirmed_merge_is_refused_if_a_pr_was_added_since() {
        let shell = MockShell::new();
        let mut app = App::new();
        let mut stack = current_stack();
        let mut extra = stack.layers[2].clone();
        extra.branch = "stack-a-layer-3".to_string();
        extra.pull_request = Some(pr(47, false));
        stack.layers.push(extra);
        app.state.stacks = vec![stack];

        let results = app
            .settle(
                &Action::MergeStarted {
                    stack_index: 0,
                    prs: vec![44, 45, 46],
                    method: MergeMethod::Merge,
                },
                &shell,
            )
            .await;

        assert!(matches!(results.as_slice(), [Action::SetError(_)]));
        assert!(shell.calls().is_empty());
    }

    #[tokio::test]
    async fn a_confirmed_merge_is_refused_if_the_stack_is_no_longer_checked_out() {
        let shell = MockShell::new();
        let mut app = App::new();
        let mut stack = current_stack();
        stack.is_current = false;
        app.state.stacks = vec![stack];

        let results = app
            .settle(
                &Action::MergeStarted {
                    stack_index: 0,
                    prs: vec![44, 45, 46],
                    method: MergeMethod::Merge,
                },
                &shell,
            )
            .await;

        assert!(matches!(results.as_slice(), [Action::SetError(_)]));
        assert!(shell.calls().is_empty());
    }

    #[test]
    fn cycling_the_method_changes_what_the_next_modal_shows() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];

        app.apply(&Action::CycleMergeMethod);
        assert_eq!(app.state.merge_method, MergeMethod::Squash);
        assert_eq!(app.state.status.as_deref(), Some("merge method: squash"));
        app.apply(&Action::CycleMergeMethod);
        assert_eq!(app.state.merge_method, MergeMethod::Rebase);

        let follow_ups = app.apply(&Action::MergeStack { stack_index: 0 });
        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a modal");
        };
        assert!(modal.body.contains("Method: rebase (--rebase)"));
    }

    #[tokio::test]
    async fn m_and_shift_m_keys_reach_the_merge_reducer() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack()];
        app.dispatch_now(vec![Action::ShowLayers(0)]);

        let cycle = app.handle_key(key(KeyCode::Char('m')));
        app.dispatch_now(cycle);
        assert_eq!(app.state.merge_method, MergeMethod::Squash);

        let merge = app.handle_key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT));
        app.dispatch_now(merge);
        let modal = app.state.confirm.as_ref().expect("merge modal shown");
        assert!(modal.danger);
        assert!(modal.body.contains("#44"));
    }

    #[test]
    fn enqueued_and_other_outcomes_have_their_own_status() {
        assert_eq!(
            merge_status(&MergeOutcome::Enqueued {
                prs: vec![1, 2],
                base: "main".to_string()
            }),
            "added #1, #2 to the merge queue for main; they merge as the queue processes them"
        );
        assert_eq!(
            merge_status(&MergeOutcome::AlreadyMerged),
            "stack was already fully merged"
        );
    }
}
