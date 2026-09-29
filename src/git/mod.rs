//! Git operations shelled out to the local `git` binary.

mod branch;
mod diff;
mod log;
mod rebase;
mod status;

#[expect(unused_imports, reason = "for the branch panel (#26)")]
pub use branch::branch;
pub use diff::diff;
#[expect(unused_imports, reason = "`log` is for the log panel (#27)")]
pub use log::{CommitEntry, log, log_range};
pub use rebase::{
    conflicted_files, has_conflict_markers, rebase_abort, rebase_continue, rebase_state, stage,
};
#[expect(unused_imports, reason = "for the status/staging panel (#24)")]
pub use status::status;
