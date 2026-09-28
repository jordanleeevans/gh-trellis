//! Domain model for a stack of branches/PRs, parsed from
//! `gh stack view --json` and supplemented by `gh pr view --json`.

mod commit;
mod detail;
mod layer;
pub mod local;
mod pull_request;
mod stack;
mod submit;
mod summary;
mod sync;
mod unstack;

pub use commit::CommitInfo;
pub use detail::{
    CheckSummary, LayerCommit, LayerDetail, PullRequestDetail, ReviewerState, hydrate_layer_detail,
};
pub use layer::Layer;
pub use pull_request::PullRequestRef;
pub use stack::Stack;
pub use submit::{SubmitEvent, SubmitLayerOutcome, SubmitOptions, submit_stack};
pub use summary::{PrCounts, StackSummary, list_stacks};
pub use sync::{SyncOptions, SyncOutcome, SyncPlan, sync_stack};
pub use unstack::{UnstackScope, unstack_stack};

#[cfg(test)]
mod tests {
    use super::*;

    const STACK_VIEW_FIXTURE: &str = include_str!("fixtures/stack_view.json");
    const STACK_VIEW_UNSUBMITTED_FIXTURE: &str =
        include_str!("fixtures/stack_view_unsubmitted.json");

    #[test]
    fn parses_a_real_stack_view_sample() {
        let stack = Stack::from_json(STACK_VIEW_FIXTURE).unwrap();

        assert_eq!(stack.trunk, "main");
        assert_eq!(stack.current_branch, "stack-demo/layer-3");
        assert_eq!(stack.layers.len(), 3);

        assert_eq!(stack.layers[0].branch, "stack-demo/layer-1");
        assert_eq!(stack.layers[0].position, 0);
        assert!(stack.layers[0].needs_rebase);
        assert_eq!(
            stack.layers[0].head.as_deref(),
            Some("60b4fd24f37bd8df27a8385ec665ca4faa56b00b")
        );
        assert_eq!(stack.layers[0].pull_request.as_ref().unwrap().number, 44);
        assert_eq!(stack.layers[0].pull_request.as_ref().unwrap().title, None);

        assert_eq!(stack.layers[2].branch, "stack-demo/layer-3");
        assert_eq!(stack.layers[2].position, 2);
        assert!(stack.layers[2].is_current);
    }

    #[test]
    fn round_trips_a_real_stack_view_sample_without_loss() {
        let stack = Stack::from_json(STACK_VIEW_FIXTURE).unwrap();

        let reserialized: serde_json::Value = serde_json::to_value(&stack).unwrap();
        let original: serde_json::Value = serde_json::from_str(STACK_VIEW_FIXTURE).unwrap();

        assert_eq!(reserialized, original);
    }

    #[test]
    fn parses_a_stack_before_any_branch_has_been_pushed() {
        let stack = Stack::from_json(STACK_VIEW_UNSUBMITTED_FIXTURE).unwrap();

        assert_eq!(stack.layers.len(), 3);
        assert_eq!(stack.layers[0].head, None);
        assert_eq!(stack.layers[0].pull_request, None);
    }

    #[test]
    fn round_trips_an_unsubmitted_stack_view_sample_without_loss() {
        let stack = Stack::from_json(STACK_VIEW_UNSUBMITTED_FIXTURE).unwrap();

        let reserialized: serde_json::Value = serde_json::to_value(&stack).unwrap();
        let original: serde_json::Value =
            serde_json::from_str(STACK_VIEW_UNSUBMITTED_FIXTURE).unwrap();

        assert_eq!(reserialized, original);
    }
}
