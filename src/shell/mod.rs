//! Abstractions for running external commands.

mod error;
#[cfg(test)]
mod mock;
mod output;
mod process;

pub use error::ShellError;
#[cfg(test)]
pub use mock::MockShell;
pub use output::ShellOutput;
pub use process::ProcessShell;

use std::path::Path;
use std::sync::Arc;

/// Runs external commands in a given working directory.
#[async_trait::async_trait]
pub trait Shell: Send + Sync {
    /// Runs `program` with `args` in `cwd`, waiting for it to complete.
    ///
    /// Returns `Err` if the binary can't be found, fails to spawn, or exits
    /// with a non-zero status.
    async fn run(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
    ) -> Result<ShellOutput, ShellError>;
}

/// Lets a shared shell (e.g. the TUI's `Arc<dyn Shell>`, cloned into each
/// background task) be passed anywhere a `&impl Shell` is expected.
#[async_trait::async_trait]
impl<S: Shell + ?Sized> Shell for Arc<S> {
    async fn run(
        &self,
        cwd: &Path,
        program: &str,
        args: &[&str],
    ) -> Result<ShellOutput, ShellError> {
        (**self).run(cwd, program, args).await
    }
}
