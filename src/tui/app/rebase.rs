//! `gh stack rebase` and the conflict flow around it: confirmation, running
//! it, detecting a stopped rebase (from this run or an earlier session),
//! editing and staging conflicted files, and continuing or aborting.
//!
//! See `stack::rebase` for what `gh stack rebase` does and how a conflict
//! is detected.

use crate::stack::{
    RebaseConflict, RebaseDriver, RebaseOutcome, RebaseScope, StackSummary, StageOutcome,
};
use crate::tui::action::Action;
use crate::tui::components::confirm::ConfirmModal;
use crate::tui::effects::Effects;
use crate::tui::keymap::{self, KeyIntent};
use crate::tui::state::RebaseOp;
use crate::tui::state::external_command::ExternalCommand;

use super::App;

/// The current label for `intent`, for messages that name a key.
fn key_label(intent: KeyIntent) -> String {
    keymap::current()
        .short_label(intent)
        .unwrap_or_else(|| "unbound".to_string())
}

impl App {
    pub(super) fn reduce_rebase(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::RebaseStack { stack_index, scope } => {
                if let Some(refusal) = self.rebase_refusal() {
                    return vec![Action::SetError(refusal)];
                }
                let stack = match self.checked_out_stack(*stack_index) {
                    Ok(stack) => stack,
                    Err(error) => return vec![Action::SetError(error)],
                };
                match rebase_modal(
                    *stack_index,
                    stack,
                    &crate::config::get().default_remote,
                    *scope,
                ) {
                    Ok(modal) => vec![Action::ShowConfirm(modal)],
                    Err(error) => vec![Action::SetError(error)],
                }
            }
            Action::RebaseStarted { stack_index, scope } => {
                if let Some(refusal) = self.rebase_refusal() {
                    return vec![Action::SetError(refusal)];
                }
                if let Err(error) = self.checked_out_stack(*stack_index) {
                    return vec![Action::SetError(error)];
                }
                self.state.rebase_in_flight = Some(RebaseOp::Rebase);
                effects.rebase_stack(crate::config::get().default_remote.clone(), *scope);
                Vec::new()
            }
            Action::RebaseFinished { result } => {
                let op = self.state.rebase_in_flight.take();
                match result {
                    Ok(RebaseOutcome::Completed) => {
                        self.state.conflict = None;
                        self.state.status = Some(
                            match op {
                                Some(RebaseOp::Continue) => {
                                    "rebase finished; push it with submit or sync"
                                }
                                _ => "stack rebased locally; push it with submit or sync",
                            }
                            .to_string(),
                        );
                        vec![Action::RefreshStacks]
                    }
                    Ok(RebaseOutcome::Conflict(conflict)) => {
                        self.state.status = Some(conflict_status(conflict, op));
                        self.state.conflict = Some(conflict.clone());
                        Vec::new()
                    }
                    Err(message) if self.state.conflict.is_some() => {
                        // e.g. `--continue` failed for a reason other than a
                        // conflict: re-read git so the view matches reality.
                        vec![Action::SetError(message.clone()), Action::LoadRebaseState]
                    }
                    Err(message) => {
                        vec![Action::SetError(message.clone()), Action::RefreshStacks]
                    }
                }
            }
            Action::LoadRebaseState => {
                // A running rebase or sync moves through mid-rebase states
                // on its own; don't mistake one for a stopped rebase.
                if !self.rebase_state_is_settling() {
                    effects.load_rebase_state();
                }
                Vec::new()
            }
            Action::RebaseStateLoaded { result } => {
                if self.rebase_state_is_settling() {
                    return Vec::new();
                }
                match result {
                    Ok(Some(conflict)) => {
                        if self.state.conflict.is_none() {
                            self.state.status = Some(format!(
                                "a rebase{} is in progress; resolve it here",
                                on_branch(conflict)
                            ));
                        }
                        self.state.conflict = Some(conflict.clone());
                        Vec::new()
                    }
                    Ok(None) => {
                        if self.state.conflict.take().is_some() {
                            self.state.status =
                                Some("the rebase is no longer in progress".to_string());
                            vec![Action::RefreshStacks]
                        } else {
                            Vec::new()
                        }
                    }
                    Err(message) => vec![Action::SetError(message.clone())],
                }
            }
            Action::EditConflictFile { path } => {
                if self.state.conflict.is_none() || self.state.rebase_in_flight.is_some() {
                    return Vec::new();
                }
                let editor = std::env::var("EDITOR").ok();
                vec![Action::RunExternal(ExternalCommand::editor(
                    editor.as_deref(),
                    path,
                    Action::LoadRebaseState,
                ))]
            }
            Action::StageConflictFile { path } => {
                if self.state.conflict.is_some() && self.state.rebase_in_flight.is_none() {
                    effects.stage_conflict_file(path.clone());
                }
                Vec::new()
            }
            Action::ConflictFileStaged { path, result } => match result {
                Ok(StageOutcome::Staged) => {
                    self.state.status = Some(format!("marked {path} resolved"));
                    vec![Action::LoadRebaseState]
                }
                Ok(StageOutcome::MarkersRemain) => vec![Action::SetError(format!(
                    "{path} still has conflict markers; edit it first ({})",
                    key_label(KeyIntent::ConflictEdit)
                ))],
                Err(message) => vec![Action::SetError(message.clone())],
            },
            Action::ContinueRebase => {
                if self.state.rebase_in_flight.is_some() {
                    return vec![Action::SetError(
                        "a rebase command is already running".into(),
                    )];
                }
                let Some(conflict) = &self.state.conflict else {
                    return Vec::new();
                };
                if !conflict.files.is_empty() {
                    return vec![Action::SetError(format!(
                        "{} file(s) still conflicted; resolve each one and mark it resolved ({}) first",
                        conflict.files.len(),
                        key_label(KeyIntent::ConflictMarkResolved)
                    ))];
                }
                let driver = conflict.driver;
                self.state.rebase_in_flight = Some(RebaseOp::Continue);
                effects.continue_rebase(driver);
                Vec::new()
            }
            Action::AbortRebase => {
                if self.state.rebase_in_flight.is_some() {
                    return vec![Action::SetError(
                        "a rebase command is already running".into(),
                    )];
                }
                match &self.state.conflict {
                    Some(conflict) => vec![Action::ShowConfirm(abort_modal(conflict))],
                    None => Vec::new(),
                }
            }
            Action::RunAbortRebase => {
                if self.state.rebase_in_flight.is_some() {
                    return vec![Action::SetError(
                        "a rebase command is already running".into(),
                    )];
                }
                if let Some(conflict) = &self.state.conflict {
                    let driver = conflict.driver;
                    self.state.rebase_in_flight = Some(RebaseOp::Abort);
                    effects.abort_rebase(driver);
                }
                Vec::new()
            }
            Action::RebaseAborted { result } => {
                self.state.rebase_in_flight = None;
                match result {
                    Ok(()) => {
                        self.state.conflict = None;
                        self.state.status = Some("rebase aborted; branches restored".to_string());
                        vec![Action::RefreshStacks]
                    }
                    Err(message) => {
                        vec![Action::SetError(message.clone()), Action::LoadRebaseState]
                    }
                }
            }
            _ => unreachable!("routed to reduce_rebase by App::apply_action"),
        }
    }

    /// Why a new rebase can't start right now, if it can't.
    fn rebase_refusal(&self) -> Option<String> {
        if self.state.rebase_in_flight.is_some() {
            Some("a rebase command is already running".to_string())
        } else if self.state.sync_in_flight {
            Some("wait for the sync to finish before rebasing".to_string())
        } else if self.state.conflict.is_some() {
            Some("a rebase is already in progress; continue or abort it first".to_string())
        } else {
            None
        }
    }

    /// The stack at `stack_index`, if it has layers and is checked out:
    /// `gh stack rebase` measures its scope from the checked-out branch.
    fn checked_out_stack(&self, stack_index: usize) -> Result<&StackSummary, String> {
        let stack = self
            .state
            .stacks
            .get(stack_index)
            .ok_or("selected stack is no longer available")?;
        if stack.layers.is_empty() {
            return Err("selected stack has no layers to rebase".to_string());
        }
        if !stack.is_current {
            return Err(format!(
                "rebase only works on the checked-out stack; check it out first ({})",
                key_label(KeyIntent::Checkout)
            ));
        }
        Ok(stack)
    }

    /// Whether git's rebase state is expected to be changing under us.
    fn rebase_state_is_settling(&self) -> bool {
        self.state.rebase_in_flight.is_some() || self.state.sync_in_flight
    }
}

fn on_branch(conflict: &RebaseConflict) -> String {
    conflict
        .branch
        .as_ref()
        .map(|branch| format!(" of {branch}"))
        .unwrap_or_default()
}

fn conflict_status(conflict: &RebaseConflict, op: Option<RebaseOp>) -> String {
    let files = conflict.files.len();
    let stopped = match op {
        Some(RebaseOp::Continue) => "rebase continued, then stopped",
        _ => "rebase stopped",
    };
    if files == 0 {
        format!(
            "{stopped}{}; continue ({}) when ready",
            on_branch(conflict),
            key_label(KeyIntent::RebaseContinue)
        )
    } else {
        format!(
            "{stopped} on a conflict{}: {files} file(s) to resolve",
            on_branch(conflict)
        )
    }
}

/// Builds the confirmation for rebasing the checked-out `stack`. Only
/// states what `gh stack rebase` was verified to do (see `stack::rebase`).
fn rebase_modal(
    stack_index: usize,
    stack: &StackSummary,
    remote: &str,
    scope: RebaseScope,
) -> Result<ConfirmModal, String> {
    let first = match scope {
        RebaseScope::Stack => 0,
        RebaseScope::Upstack => stack
            .layers
            .iter()
            .position(|layer| layer.is_current)
            .ok_or("no layer of this stack is checked out")?,
    };
    let branches: Vec<&str> = stack.layers[first..]
        .iter()
        .filter(|layer| !layer.is_merged)
        .map(|layer| layer.branch.as_str())
        .collect();
    let parent = match first {
        0 => stack.trunk.as_str(),
        n => stack.layers[n - 1].branch.as_str(),
    };

    let (title, what) = match scope {
        RebaseScope::Stack => (
            format!("Rebase {}?", stack.label),
            format!(
                "Rebase every layer onto its parent, bottom to top, starting on {parent}: {}.",
                branches.join(" → ")
            ),
        ),
        RebaseScope::Upstack => (
            format!("Rebase upstack from {}?", stack.layers[first].branch),
            format!(
                "Rebase the checked-out layer and those above it, starting on {parent}: {}.",
                branches.join(" → ")
            ),
        ),
    };
    let body = [
        format!("Fetch {remote} first, and fast-forward branches that are behind it."),
        what,
        "Merged layers are skipped. Nothing is pushed; use submit or sync afterwards.".to_string(),
        "If a layer conflicts, the rebase stops there and trellis lists the conflicted \
         files so you can resolve them, then continue or abort (abort restores every branch)."
            .to_string(),
    ]
    .join("\n");

    let label = match scope {
        RebaseScope::Stack => "Rebase",
        RebaseScope::Upstack => "Rebase upstack",
    };
    Ok(ConfirmModal::new(title, body, label, false)
        .on_confirm(Action::RebaseStarted { stack_index, scope }))
}

/// Builds the confirmation for aborting `conflict`'s rebase.
fn abort_modal(conflict: &RebaseConflict) -> ConfirmModal {
    let body = match conflict.driver {
        RebaseDriver::GhStack => "Runs `gh stack rebase --abort`: stops the rebase and resets \
             every branch in the stack to where it was before the rebase started."
            .to_string(),
        RebaseDriver::GhStackModify => "Runs `gh stack modify --abort`: abandons the \
             restructure and restores the stack to how it was before `gh stack modify`."
            .to_string(),
        RebaseDriver::Git => format!(
            "Runs `git rebase --abort`: stops the rebase and puts {} back where it was.",
            conflict.branch.as_deref().unwrap_or("the branch")
        ),
    };
    ConfirmModal::new(
        "Abort rebase?",
        format!("{body}\nAny conflicts you've resolved so far are discarded."),
        "Abort rebase",
        false,
    )
    .on_confirm(Action::RunAbortRebase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::ConflictedFile;
    use crate::tui::app::test_support::*;

    fn ok(stdout: &str) -> Result<crate::shell::ShellOutput, ShellError> {
        Ok(crate::shell::ShellOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn current_stack(label: &str, layers: usize, current_layer: usize) -> StackSummary {
        let mut stack = StackSummary {
            is_current: true,
            ..stack_summary(label, layers)
        };
        stack.layers[current_layer].is_current = true;
        stack
    }

    fn conflict(driver: RebaseDriver, files: &[&str]) -> RebaseConflict {
        RebaseConflict {
            driver,
            branch: Some("s-layer-1".to_string()),
            files: files
                .iter()
                .map(|path| ConflictedFile {
                    path: path.to_string(),
                    has_markers: true,
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn rebase_asks_for_confirmation_listing_the_layers() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![current_stack("s", 3, 1)];

        let stack = app
            .settle(
                &Action::RebaseStack {
                    stack_index: 0,
                    scope: RebaseScope::Stack,
                },
                &shell,
            )
            .await;
        let upstack = app.apply(&Action::RebaseStack {
            stack_index: 0,
            scope: RebaseScope::Upstack,
        });

        let [Action::ShowConfirm(stack)] = stack.as_slice() else {
            panic!("expected a confirm modal, got {stack:?}");
        };
        assert!(!stack.danger);
        assert!(stack.body.contains("s-layer-0 → s-layer-1 → s-layer-2"));
        assert!(stack.body.contains("starting on main"));
        let [Action::ShowConfirm(upstack)] = upstack.as_slice() else {
            panic!("expected a confirm modal, got {upstack:?}");
        };
        assert_eq!(upstack.title, "Rebase upstack from s-layer-1?");
        assert!(
            upstack
                .body
                .contains("starting on s-layer-0: s-layer-1 → s-layer-2")
        );
        assert!(shell.calls().is_empty(), "nothing runs before confirming");
    }

    #[test]
    fn confirming_starts_the_rebase_with_the_chosen_scope() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack("s", 2, 0)];
        let shown = app.apply(&Action::RebaseStack {
            stack_index: 0,
            scope: RebaseScope::Upstack,
        });
        app.apply(&shown[0]);

        let accepted = app.apply(&Action::ConfirmAccept);

        assert!(matches!(
            accepted.as_slice(),
            [Action::RebaseStarted {
                stack_index: 0,
                scope: RebaseScope::Upstack
            }]
        ));
    }

    #[test]
    fn rebase_is_refused_for_a_stack_that_is_not_checked_out() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("s", 2)];

        let follow_ups = app.apply(&Action::RebaseStack {
            stack_index: 0,
            scope: RebaseScope::Stack,
        });

        let [Action::SetError(message)] = follow_ups.as_slice() else {
            panic!("expected an error, got {follow_ups:?}");
        };
        assert!(message.contains("checked-out stack"));
    }

    #[test]
    fn rebase_is_refused_while_a_rebase_is_stopped_or_running() {
        let mut app = App::new();
        app.state.stacks = vec![current_stack("s", 2, 0)];
        app.state.conflict = Some(conflict(RebaseDriver::GhStack, &["a"]));
        let rebase = Action::RebaseStack {
            stack_index: 0,
            scope: RebaseScope::Stack,
        };

        assert!(matches!(
            app.apply(&rebase).as_slice(),
            [Action::SetError(_)]
        ));

        app.state.conflict = None;
        app.state.rebase_in_flight = Some(RebaseOp::Rebase);
        assert!(matches!(
            app.apply(&rebase).as_slice(),
            [Action::SetError(_)]
        ));
    }

    #[tokio::test]
    async fn a_clean_rebase_reports_success_and_refreshes() {
        let shell = MockShell::new().when(
            "gh",
            &["stack", "rebase", "--remote", "origin"],
            ok("All branches in stack rebased locally"),
        );
        let mut app = App::new();
        app.state.stacks = vec![current_stack("s", 2, 1)];

        let results = app
            .settle(
                &Action::RebaseStarted {
                    stack_index: 0,
                    scope: RebaseScope::Stack,
                },
                &shell,
            )
            .await;
        assert_eq!(app.state.rebase_in_flight, Some(RebaseOp::Rebase));
        let [
            Action::RebaseFinished {
                result: Ok(RebaseOutcome::Completed),
            },
        ] = results.as_slice()
        else {
            panic!("unexpected results: {results:?}");
        };

        let follow_ups = app.apply(&results[0]);
        assert_eq!(app.state.rebase_in_flight, None);
        assert!(app.state.conflict.is_none());
        assert!(app.state.status.as_deref().unwrap().contains("rebased"));
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[test]
    fn a_conflict_enters_the_conflict_view_instead_of_reporting_an_outcome() {
        let mut app = App::new();
        app.state.rebase_in_flight = Some(RebaseOp::Rebase);

        let follow_ups = app.apply(&Action::RebaseFinished {
            result: Ok(RebaseOutcome::Conflict(conflict(
                RebaseDriver::GhStack,
                &["a.rs", "b.rs"],
            ))),
        });

        assert!(follow_ups.is_empty());
        assert!(app.state.error.is_none());
        assert_eq!(app.state.conflict.as_ref().unwrap().files.len(), 2);
        assert_eq!(
            app.state.status.as_deref(),
            Some("rebase stopped on a conflict of s-layer-1: 2 file(s) to resolve")
        );
    }

    #[test]
    fn a_failed_rebase_is_an_error() {
        let mut app = App::new();
        app.state.rebase_in_flight = Some(RebaseOp::Rebase);

        let follow_ups = app.apply(&Action::RebaseFinished {
            result: Err("rebase stack: could not fetch".to_string()),
        });

        assert!(app.state.conflict.is_none());
        assert!(matches!(
            follow_ups.as_slice(),
            [Action::SetError(_), Action::RefreshStacks]
        ));
    }

    #[tokio::test]
    async fn refresh_also_checks_for_an_interrupted_rebase() {
        let mut app = App::new();

        let follow_ups = app.apply(&Action::RefreshStacks);

        assert!(
            follow_ups
                .iter()
                .any(|action| matches!(action, Action::LoadRebaseState))
        );
    }

    #[test]
    fn a_rebase_found_on_startup_opens_the_conflict_view() {
        let mut app = App::new();

        app.dispatch_now(vec![Action::RebaseStateLoaded {
            result: Ok(Some(conflict(RebaseDriver::Git, &["a.rs"]))),
        }]);

        assert!(app.state.conflict.is_some());
        assert!(app.state.status.as_deref().unwrap().contains("in progress"));
    }

    #[test]
    fn rebase_state_is_ignored_while_a_rebase_or_sync_runs() {
        for (rebase, sync) in [(Some(RebaseOp::Continue), false), (None, true)] {
            let mut app = App::new();
            app.state.rebase_in_flight = rebase;
            app.state.sync_in_flight = sync;

            app.apply(&Action::RebaseStateLoaded {
                result: Ok(Some(conflict(RebaseDriver::Git, &["a.rs"]))),
            });

            assert!(app.state.conflict.is_none());
        }
    }

    #[test]
    fn a_rebase_finished_elsewhere_closes_the_conflict_view() {
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::Git, &[]));

        let follow_ups = app.apply(&Action::RebaseStateLoaded { result: Ok(None) });

        assert!(app.state.conflict.is_none());
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[test]
    fn editing_a_file_hands_the_terminal_to_the_editor_then_reloads() {
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::Git, &["a.rs"]));

        let follow_ups = app.apply(&Action::EditConflictFile {
            path: "a.rs".to_string(),
        });
        let [Action::RunExternal(command)] = follow_ups.as_slice() else {
            panic!("expected an external command, got {follow_ups:?}");
        };
        assert_eq!(command.args.last().map(String::as_str), Some("a.rs"));
        assert!(matches!(*command.then, Action::LoadRebaseState));

        app.apply(&follow_ups[0]);
        assert!(app.state.external_command.is_some());
    }

    #[test]
    fn continue_is_refused_while_files_are_still_conflicted() {
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::GhStack, &["a.rs"]));

        let follow_ups = app.apply(&Action::ContinueRebase);

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
        assert_eq!(app.state.rebase_in_flight, None);
    }

    #[tokio::test]
    async fn continue_uses_the_driver_that_owns_the_rebase() {
        let shell = MockShell::new().when("gh", &["stack", "rebase", "--continue"], ok(""));
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::GhStack, &[]));

        let results = app.settle(&Action::ContinueRebase, &shell).await;

        assert_eq!(app.state.rebase_in_flight, Some(RebaseOp::Continue));
        assert!(matches!(
            results.as_slice(),
            [Action::RebaseFinished {
                result: Ok(RebaseOutcome::Completed)
            }]
        ));
        app.apply(&results[0]);
        assert!(app.state.conflict.is_none());
        assert_eq!(
            app.state.status.as_deref(),
            Some("rebase finished; push it with submit or sync")
        );
    }

    #[tokio::test]
    async fn abort_confirms_then_runs_and_restores() {
        let shell = MockShell::new().when("gh", &["stack", "rebase", "--abort"], ok(""));
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::GhStack, &["a.rs"]));

        let shown = app.apply(&Action::AbortRebase);
        let [Action::ShowConfirm(modal)] = shown.as_slice() else {
            panic!("expected a confirm modal, got {shown:?}");
        };
        assert!(modal.body.contains("gh stack rebase --abort"));
        assert!(shell.calls().is_empty());
        app.apply(&shown[0]);
        let accepted = app.apply(&Action::ConfirmAccept);
        assert!(matches!(accepted.as_slice(), [Action::RunAbortRebase]));

        let results = app.settle(&Action::RunAbortRebase, &shell).await;
        let follow_ups = app.apply(&results[0]);

        assert!(app.state.conflict.is_none());
        assert_eq!(app.state.rebase_in_flight, None);
        assert_eq!(
            app.state.status.as_deref(),
            Some("rebase aborted; branches restored")
        );
        assert!(matches!(follow_ups.as_slice(), [Action::RefreshStacks]));
    }

    #[test]
    fn staging_a_file_with_markers_left_is_an_error() {
        let mut app = App::new();
        app.state.conflict = Some(conflict(RebaseDriver::Git, &["a.rs"]));

        let follow_ups = app.apply(&Action::ConflictFileStaged {
            path: "a.rs".to_string(),
            result: Ok(StageOutcome::MarkersRemain),
        });
        let staged = app.apply(&Action::ConflictFileStaged {
            path: "a.rs".to_string(),
            result: Ok(StageOutcome::Staged),
        });

        let [Action::SetError(message)] = follow_ups.as_slice() else {
            panic!("expected an error, got {follow_ups:?}");
        };
        assert!(message.contains("conflict markers"));
        assert!(matches!(staged.as_slice(), [Action::LoadRebaseState]));
    }

    /// The acceptance scenario, driven through the real reducer and effects
    /// against a real repository stopped on a real conflict (plain `git
    /// rebase`: `gh stack` can't run offline): detect it, fail to continue
    /// early, resolve the file, mark it resolved, and continue to the end.
    #[tokio::test]
    async fn resolves_a_real_conflict_end_to_end() {
        use crate::test_fixtures::conflicting_rebase_repo;

        let dir = conflicting_rebase_repo();
        let repo = dir.path();
        let mut app = App::new();

        let loaded = app.settle_in(&Action::LoadRebaseState, repo).await;
        app.dispatch_now(loaded);
        let conflict = app.state.conflict.clone().expect("conflict detected");
        assert_eq!(conflict.driver, RebaseDriver::Git);
        assert_eq!(conflict.files[0].path, "shared.txt");
        assert!(conflict.files[0].has_markers);

        // Continuing too early is refused without running anything.
        assert!(matches!(
            app.apply(&Action::ContinueRebase).as_slice(),
            [Action::SetError(_)]
        ));

        // Marking it resolved while markers remain is refused.
        let staged = app
            .settle_in(
                &Action::StageConflictFile {
                    path: "shared.txt".to_string(),
                },
                repo,
            )
            .await;
        assert!(matches!(
            app.apply(&staged[0]).as_slice(),
            [Action::SetError(_)]
        ));

        // "Edit" the file, as the editor would, then mark it resolved.
        std::fs::write(repo.join("shared.txt"), "resolved\n").unwrap();
        let staged = app
            .settle_in(
                &Action::StageConflictFile {
                    path: "shared.txt".to_string(),
                },
                repo,
            )
            .await;
        let reload = app.apply(&staged[0]);
        assert!(matches!(reload.as_slice(), [Action::LoadRebaseState]));
        let loaded = app.settle_in(&reload[0], repo).await;
        app.dispatch_now(loaded);
        assert!(app.state.conflict.as_ref().unwrap().files.is_empty());

        let finished = app.settle_in(&Action::ContinueRebase, repo).await;
        assert!(matches!(
            finished.as_slice(),
            [Action::RebaseFinished {
                result: Ok(RebaseOutcome::Completed)
            }]
        ));
        app.apply(&finished[0]);
        assert!(app.state.conflict.is_none());

        let after = app.settle_in(&Action::LoadRebaseState, repo).await;
        assert!(matches!(
            after.as_slice(),
            [Action::RebaseStateLoaded { result: Ok(None) }]
        ));
    }

    #[tokio::test]
    async fn aborts_a_real_conflict() {
        use crate::test_fixtures::{FIXTURE_FEATURE_TEXT, conflicting_rebase_repo};

        let dir = conflicting_rebase_repo();
        let repo = dir.path();
        let mut app = App::new();
        let loaded = app.settle_in(&Action::LoadRebaseState, repo).await;
        app.dispatch_now(loaded);
        assert!(app.state.conflict.is_some());

        let aborted = app.settle_in(&Action::RunAbortRebase, repo).await;
        app.apply(&aborted[0]);

        assert!(app.state.conflict.is_none());
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            FIXTURE_FEATURE_TEXT
        );
        let after = app.settle_in(&Action::LoadRebaseState, repo).await;
        assert!(matches!(
            after.as_slice(),
            [Action::RebaseStateLoaded { result: Ok(None) }]
        ));
    }
}
