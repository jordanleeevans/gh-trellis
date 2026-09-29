//! Builds the danger confirmation shown before `gh stack merge` runs.
//!
//! Merging is irreversible, so this is a `danger` [`ConfirmModal`] (typed
//! phrase required). The body lists, in merge order, exactly the pull
//! requests [`MergePlan`] says gh-stack will merge, plus the method, and
//! states only what `stack::merge` verified in gh-stack's source.

use crate::stack::{MergeCandidate, MergeMethod, MergePlan};
use crate::tui::action::Action;
use crate::tui::keymap::{self, KeyIntent};

use super::confirm::ConfirmModal;

pub(crate) fn merge_modal(
    stack_index: usize,
    stack_label: &str,
    plan: &MergePlan,
    method: MergeMethod,
) -> ConfirmModal {
    let count = plan.prs.len();
    let mut lines = vec![format!(
        "Merges into {}, bottom to top, as one all-or-nothing GitHub stack merge:",
        plan.trunk
    )];
    lines.extend(
        plan.prs
            .iter()
            .enumerate()
            .map(|(index, pr)| format!("  {}. {}", index + 1, describe(pr))),
    );

    let toggle = keymap::current()
        .short_label(KeyIntent::CycleMergeMethod)
        .map(|key| format!(" (esc, then {key} to change)"))
        .unwrap_or_default();
    lines.push(format!(
        "Method: {} ({}){toggle}",
        method.name(),
        method.flag()
    ));

    if !plan.already_merged.is_empty() {
        let merged: Vec<String> = plan
            .already_merged
            .iter()
            .map(|pr| format!("#{}", pr.number))
            .collect();
        lines.push(format!("Already merged, skipped: {}", merged.join(", ")));
    }
    if !plan.not_submitted.is_empty() {
        lines.push(format!(
            "No PR, not merged: {}",
            plan.not_submitted.join(", ")
        ));
    }
    lines.push(
        "GitHub checks required reviews, checks and rules as it merges; if any PR fails them, nothing merges."
            .to_string(),
    );
    lines.push(format!(
        "If {} uses a merge queue, the PRs are queued instead (the queue picks the method).",
        plan.trunk
    ));
    lines.push("This cannot be undone.".to_string());

    ConfirmModal::new(
        format!(
            "Merge {count} pull request{} from {stack_label}?",
            if count == 1 { "" } else { "s" }
        ),
        lines.join("\n"),
        "Merge",
        true,
    )
    .on_confirm(Action::MergeStarted {
        stack_index,
        prs: plan.pr_numbers(),
        method,
    })
}

/// `#44 Title (branch)`, or `#44 (branch)` when the title isn't known.
fn describe(pr: &MergeCandidate) -> String {
    match pr.title.as_deref().filter(|title| !title.is_empty()) {
        Some(title) => format!("#{} {title} ({})", pr.number, pr.branch),
        None => format!("#{} ({})", pr.number, pr.branch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(number: u64, title: Option<&str>) -> MergeCandidate {
        MergeCandidate {
            number,
            title: title.map(str::to_string),
            branch: format!("layer-{number}"),
        }
    }

    fn plan() -> MergePlan {
        MergePlan {
            trunk: "main".to_string(),
            prs: vec![candidate(44, Some("First")), candidate(45, None)],
            already_merged: vec![candidate(43, Some("Old"))],
            not_submitted: vec!["layer-wip".to_string()],
        }
    }

    #[test]
    fn body_names_every_pr_in_order_with_the_method() {
        let modal = merge_modal(3, "stack-a", &plan(), MergeMethod::Squash);

        assert!(modal.danger);
        assert!(!modal.can_confirm());
        assert_eq!(modal.title, "Merge 2 pull requests from stack-a?");
        let first = modal.body.find("1. #44 First (layer-44)").unwrap();
        let second = modal.body.find("2. #45 (layer-45)").unwrap();
        assert!(first < second);
        assert!(
            modal
                .body
                .contains("Method: squash (--squash) (esc, then m")
        );
        assert!(modal.body.contains("Already merged, skipped: #43"));
        assert!(modal.body.contains("No PR, not merged: layer-wip"));
        assert!(modal.body.contains("nothing merges"));
    }

    #[test]
    fn confirming_carries_the_exact_prs_and_method() {
        let mut modal = merge_modal(3, "stack-a", &plan(), MergeMethod::Rebase);
        for c in "yes".chars() {
            modal.push_char(c);
        }

        let Some(Action::MergeStarted {
            stack_index,
            prs,
            method,
        }) = modal.into_confirmed_action()
        else {
            panic!("expected MergeStarted");
        };
        assert_eq!(stack_index, 3);
        assert_eq!(prs, [44, 45]);
        assert_eq!(method, MergeMethod::Rebase);
    }

    #[test]
    fn a_single_pr_title_is_singular() {
        let mut plan = plan();
        plan.prs.truncate(1);
        let modal = merge_modal(0, "s", &plan, MergeMethod::Merge);
        assert_eq!(modal.title, "Merge 1 pull request from s?");
    }
}
