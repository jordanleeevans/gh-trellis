//! The gh-stack domain: stacks read from gh-stack's local tracking file,
//! hydrated with `gh pr view --json`, and the stack operations (submit,
//! sync, rebase, merge, unstack).

mod commit;
mod detail;
mod layer;
pub mod local;
mod merge;
mod pull_request;
mod rebase;
mod submit;
mod summary;
mod sync;
mod unstack;

#[cfg(test)]
pub use detail::{CheckSummary, LayerCommit, PullRequestDetail, ReviewerState};
pub use detail::{LayerDetail, hydrate_layer_detail};
pub use layer::Layer;
pub use merge::{
    MergeCandidate, MergeFailure, MergeMethod, MergeOutcome, MergePlan, MergeRefusal, merge_plan,
    merge_stack,
};
#[cfg(test)]
pub use pull_request::PullRequestRef;
pub use rebase::{
    ConflictedFile, RebaseConflict, RebaseDriver, RebaseOutcome, RebaseScope, StageOutcome,
    abort_rebase, continue_rebase, interrupted_rebase, rebase_stack, stage_resolved,
};
pub use submit::{SubmitEvent, SubmitLayerOutcome, SubmitOptions, submit_stack};
pub use summary::{StackSummary, list_stacks};
pub use sync::{SyncOptions, SyncOutcome, SyncPlan, sync_stack};
pub use unstack::{UnstackScope, unstack_stack};
