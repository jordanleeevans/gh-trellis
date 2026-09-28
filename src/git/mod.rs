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
pub use rebase::rebase;
pub use status::status;
pub use version::version;
