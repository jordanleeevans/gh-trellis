//! `gh stack modify`: restructure the checked-out stack (drop, fold, insert,
//! reorder and rename branches) in the extension's own interactive TUI.
//!
//! trellis doesn't reimplement any of that. It confirms, then hands the
//! terminal over through [`Action::RunExternal`] (see `external.rs`) and
//! refreshes when the command exits. A native in-TUI editor is the stretch
//! goal of #20 and is deliberately not built.
//!
//! What `gh stack modify` does, verified from `gh stack modify --help` and
//! `cmd/modify.go` in github/gh-stack
//! (`gh api repos/github/gh-stack/contents/cmd/modify.go`):
//!
//! - It acts on the *current* stack (`loadStack(cfg, "")`, the stack of the
//!   checked-out branch), so trellis refuses any other stack.
//! - It needs a TTY (`cfg.IsInteractive()`, else "modify requires an
//!   interactive terminal"): it opens a bubbletea TUI on the alt screen.
//!   Changes are staged there and applied together on Ctrl+S; quitting
//!   without applying changes nothing.
//! - Preconditions, checked before the TUI opens: no earlier modify session
//!   or rebase in progress, a clean working tree, no PR in the merge queue,
//!   a linear stack. Otherwise it prints an error and exits non-zero.
//! - Applying rewrites local branches only (rebases, cherry-picks for folds,
//!   renames) and never pushes. Remote branches and PRs are untouched; the
//!   extension says to run `gh stack submit` afterwards to push and to
//!   update the PRs and recreate the stack on GitHub. A dropped branch's PR
//!   stays open.
//! - It can stop on a conflict. Its state lives in its own state file, not
//!   just git's rebase state, and is finished with `gh stack modify
//!   --continue` or `--abort`. Trellis's conflict view detects that state
//!   file and drives those commands (`RebaseDriver::GhStackModify`).
//!
//! One `then` action only, so the follow-up is `RefreshStacks`: it reloads
//! the stacks and also re-checks for a rebase left stopped (see
//! `refresh.rs`), which covers `LoadRebaseState` too.

use crate::stack::StackSummary;
use crate::tui::action::Action;
use crate::tui::components::confirm::ConfirmModal;
use crate::tui::keymap::{self, KeyIntent};
use crate::tui::state::external_command::ExternalCommand;

use super::App;

impl App {
    pub(super) fn reduce_modify(&mut self, action: &Action) -> Vec<Action> {
        match action {
            Action::ModifyStack { stack_index } => match self.modify_target(*stack_index) {
                Ok(stack) => vec![Action::ShowConfirm(modify_modal(stack, *stack_index))],
                Err(error) => vec![Action::SetError(error)],
            },
            Action::ModifyStarted { stack_index } => match self.modify_target(*stack_index) {
                Ok(_) => vec![Action::RunExternal(modify_command())],
                Err(error) => vec![Action::SetError(error)],
            },
            _ => unreachable!("routed to reduce_modify by App::apply_action"),
        }
    }

    /// The stack at `stack_index` if `gh stack modify` may run on it now.
    fn modify_target(&self, stack_index: usize) -> Result<&StackSummary, String> {
        let stack = self
            .state
            .stacks
            .get(stack_index)
            .ok_or("selected stack is no longer available")?;
        // `gh stack modify` operates on the stack of the checked-out branch.
        if !stack.is_current {
            let key = keymap::current()
                .short_label(KeyIntent::Checkout)
                .unwrap_or_else(|| "unbound".to_string());
            return Err(format!(
                "modify only works on the checked-out stack; check it out first ({key})"
            ));
        }
        if stack.layers.is_empty() {
            return Err("selected stack has no layers to restructure".to_string());
        }
        let state = &self.state;
        if state.sync_in_flight || state.merge_in_flight || state.rebase_in_flight.is_some() {
            return Err("wait for the running stack command to finish first".to_string());
        }
        if state.conflict.is_some() {
            return Err("a rebase is in progress; continue or abort it first".to_string());
        }
        Ok(stack)
    }
}

fn modify_command() -> ExternalCommand {
    ExternalCommand::new(
        "gh stack modify",
        "gh",
        vec!["stack".to_string(), "modify".to_string()],
        Action::RefreshStacks,
    )
}

fn modify_modal(stack: &StackSummary, stack_index: usize) -> ConfirmModal {
    ConfirmModal::new(
        format!("Restructure {} with gh stack modify?", stack.label),
        "trellis hands the terminal to gh stack modify, where you can drop, fold, \
         insert, reorder and rename branches. Changes apply together on Ctrl+S; \
         quitting there changes nothing. Applying rewrites local branches only: \
         nothing is pushed and no pull request changes until you submit \
         afterwards. A dropped branch's PR stays open. It needs a clean working \
         tree. If it stops on a conflict, trellis opens its conflict view, \
         which continues or aborts through `gh stack modify`.",
        "Open gh stack modify",
        false,
    )
    .on_confirm(Action::ModifyStarted { stack_index })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    fn app_with_stack(current: bool) -> App {
        let mut app = App::new();
        let mut stack = stack_summary("stack-a", 2);
        stack.is_current = current;
        app.state.stacks = vec![stack];
        app
    }

    #[test]
    fn a_stack_that_is_not_checked_out_is_refused() {
        let mut app = app_with_stack(false);

        let follow_ups = app.apply(&Action::ModifyStack { stack_index: 0 });

        assert!(
            matches!(follow_ups.as_slice(), [Action::SetError(m)] if m.contains("checked-out"))
        );
        assert!(app.state.external_command.is_none());
    }

    #[test]
    fn a_missing_stack_or_a_running_command_is_refused() {
        let mut app = app_with_stack(true);
        let follow_ups = app.apply(&Action::ModifyStack { stack_index: 5 });
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));

        app.state.sync_in_flight = true;
        let follow_ups = app.apply(&Action::ModifyStack { stack_index: 0 });
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[test]
    fn the_checked_out_stack_shows_a_non_danger_confirmation() {
        let mut app = app_with_stack(true);

        let follow_ups = app.apply(&Action::ModifyStack { stack_index: 0 });

        let [Action::ShowConfirm(modal)] = follow_ups.as_slice() else {
            panic!("expected a confirm modal, got {follow_ups:?}");
        };
        assert!(!modal.danger);
        assert!(modal.body.contains("nothing is pushed"));
        assert!(matches!(
            modal.clone().into_confirmed_action(),
            Some(Action::ModifyStarted { stack_index: 0 })
        ));
    }

    #[test]
    fn confirming_runs_gh_stack_modify_then_refreshes() {
        let mut app = app_with_stack(true);

        let follow_ups = app.apply(&Action::ModifyStarted { stack_index: 0 });

        let [Action::RunExternal(command)] = follow_ups.as_slice() else {
            panic!("expected an external command, got {follow_ups:?}");
        };
        assert_eq!(command.program, "gh");
        assert_eq!(command.args, ["stack", "modify"]);
        assert!(matches!(*command.then, Action::RefreshStacks));
    }

    #[test]
    fn confirming_is_refused_if_the_stack_stopped_being_checked_out() {
        let mut app = app_with_stack(false);

        let follow_ups = app.apply(&Action::ModifyStarted { stack_index: 0 });

        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }
}
