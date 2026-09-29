//! Git operations shelled out to the local `git` binary.

mod branch;
mod diff;
mod log;
mod rebase;
mod status;
mod version;

pub use branch::branch;
pub use diff::diff;
pub use log::{CommitEntry, log, log_range};
pub use rebase::{
    conflicted_files, has_conflict_markers, rebase, rebase_abort, rebase_continue, rebase_state,
    stage,
};
pub use status::status;
pub use version::version;
