use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::{Shell, ShellError, ShellOutput};

type Call = (String, Vec<String>);

/// A [`Shell`] test double that returns canned responses instead of running
/// real processes.
///
/// Responses are registered per `(program, args)` pair with [`MockShell::when`]
/// and are consumed the first time a matching call is made; calling `run`
/// again with the same `(program, args)` without registering another
/// response panics, as does calling with a pair that was never registered.
///
/// Clones share the same responses and call log, so a clone can be handed
/// to code that needs an owned `Arc<dyn Shell>` while the test keeps one to
/// inspect.
#[derive(Default, Clone)]
pub struct MockShell {
    responses: Arc<Mutex<HashMap<Call, Result<ShellOutput, ShellError>>>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl MockShell {
    /// Creates a `MockShell` with no responses registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the result to return the next time `run` is called with
    /// `program` and `args`.
    pub fn when(
        self,
        program: &str,
        args: &[&str],
        result: Result<ShellOutput, ShellError>,
    ) -> Self {
        let key = to_call(program, args);
        self.responses.lock().unwrap().insert(key, result);
        self
    }

    /// Returns every `(program, args)` pair `run` was called with, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Shell for MockShell {
    async fn run(
        &self,
        _cwd: &Path,
        program: &str,
        args: &[&str],
    ) -> Result<ShellOutput, ShellError> {
        let key = to_call(program, args);
        self.calls.lock().unwrap().push(key.clone());

        self.responses
            .lock()
            .unwrap()
            .remove(&key)
            .unwrap_or_else(|| {
                panic!(
                    "MockShell: no response registered for `{} {}`",
                    key.0,
                    key.1.join(" ")
                )
            })
    }
}

/// Converts a program and its arguments into a `Call` tuple.
fn to_call(program: &str, args: &[&str]) -> Call {
    (
        program.to_owned(),
        args.iter().map(|arg| arg.to_string()).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[tokio::test]
    async fn returns_the_registered_response_for_a_matching_call() {
        let shell = MockShell::new().when(
            "git",
            &["--version"],
            Ok(ShellOutput {
                stdout: "git version 2.43.0".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let output = shell
            .run(env::current_dir().unwrap().as_path(), "git", &["--version"])
            .await
            .unwrap();

        assert_eq!(output.stdout, "git version 2.43.0");
    }

    #[tokio::test]
    async fn returns_a_registered_error() {
        let shell = MockShell::new().when(
            "gh",
            &["--version"],
            Err(ShellError::BinaryNotFound("gh".to_string())),
        );

        let result = shell
            .run(env::current_dir().unwrap().as_path(), "gh", &["--version"])
            .await;

        assert!(matches!(result, Err(ShellError::BinaryNotFound(program)) if program == "gh"));
    }

    #[tokio::test]
    async fn records_calls_in_order() {
        let shell = MockShell::new()
            .when(
                "git",
                &["--version"],
                Ok(ShellOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            )
            .when(
                "gh",
                &["--version"],
                Ok(ShellOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            );

        let cwd = env::current_dir().unwrap();
        shell.run(&cwd, "git", &["--version"]).await.unwrap();
        shell.run(&cwd, "gh", &["--version"]).await.unwrap();

        assert_eq!(
            shell.calls(),
            vec![
                ("git".to_string(), vec!["--version".to_string()]),
                ("gh".to_string(), vec!["--version".to_string()]),
            ]
        );
    }

    #[tokio::test]
    #[should_panic(expected = "no response registered")]
    async fn panics_on_an_unregistered_call() {
        let shell = MockShell::new();

        let _ = shell
            .run(env::current_dir().unwrap().as_path(), "git", &["--version"])
            .await;
    }
}
