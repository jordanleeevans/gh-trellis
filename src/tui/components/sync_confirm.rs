//! Builds the confirmation modal shown before `gh stack sync` runs.
//!
//! Sync is not irreversible (a conflicting rebase is rolled back, and pushes
//! use `--force-with-lease`), so this is a plain, non-danger
//! [`ConfirmModal`]; the body lists what will happen so the user can see
//! which branches are about to be rebased and force-pushed.

use crate::stack::{StackSummary, SyncOptions, SyncPlan};
use crate::tui::action::Action;

use super::confirm::ConfirmModal;

pub(crate) fn sync_modal(
    stack_index: usize,
    stack: &StackSummary,
    remote: &str,
    options: SyncOptions,
) -> ConfirmModal {
    let plan = SyncPlan::for_stack(stack, options);
    let mut lines = vec![
        format!(
            "Fetch {remote} and fast-forward {} (nothing is rebased if it hasn't moved).",
            plan.trunk
        ),
        format!(
            "Rebase onto updated parents: {}",
            list_or_none(&plan.rebase)
        ),
        format!(
            "Push (force-with-lease, atomic): {}",
            list_or_none(&plan.push)
        ),
        "Then sync PR state and link open PRs into the stack on GitHub.".to_string(),
    ];

    if options.prune {
        lines.push(format!(
            "Prune (--prune) local merged branches: {}",
            list_or_none(&plan.prune)
        ));
    } else {
        lines.push("Prune: off (merged branches are kept).".to_string());
    }

    if plan.needs_rebase() {
        lines.push(format!(
            "Warning: {} layer(s) need a rebase; a conflict aborts the sync and restores your branches.",
            plan.rebase.len()
        ));
    }
    if !plan.without_pull_request.is_empty() {
        lines.push(format!(
            "Note: no PR yet for {}; sync never opens PRs (use submit).",
            plan.without_pull_request.join(", ")
        ));
    }
    lines.push(
        "If your local and remote stacks have diverged, sync stops without pushing anything."
            .to_string(),
    );

    ConfirmModal::new(
        format!("Sync {}?", stack.label),
        lines.join("\n"),
        "Sync",
        false,
    )
    .on_confirm(Action::SyncStarted { stack_index })
}

fn list_or_none(branches: &[String]) -> String {
    if branches.is_empty() {
        "none".to_string()
    } else {
        branches.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::stack_summary;

    #[test]
    fn body_names_the_branches_to_rebase_and_push() {
        let mut stack = stack_summary("s", 3);
        stack.layers[1].needs_rebase = true;

        let modal = sync_modal(2, &stack, "origin", SyncOptions::default());

        assert!(!modal.danger);
        assert!(modal.can_confirm());
        assert!(
            modal
                .body
                .contains("Rebase onto updated parents: s-layer-1, s-layer-2")
        );
        assert!(
            modal
                .body
                .contains("Push (force-with-lease, atomic): s-layer-0, s-layer-1, s-layer-2")
        );
        assert!(modal.body.contains("Warning: 2 layer(s) need a rebase"));
        assert!(modal.body.contains("Prune: off"));
        assert!(matches!(
            modal.into_confirmed_action(),
            Some(Action::SyncStarted { stack_index: 2 })
        ));
    }

    #[test]
    fn body_omits_the_rebase_warning_when_up_to_date_and_shows_prune_targets() {
        let mut stack = stack_summary("s", 2);
        stack.layers[0].is_merged = true;

        let modal = sync_modal(0, &stack, "origin", SyncOptions { prune: true });

        assert!(!modal.body.contains("Warning"));
        assert!(
            modal
                .body
                .contains("Prune (--prune) local merged branches: s-layer-0")
        );
    }
}
