use std::path::Path;

use crate::shell::{Shell, ShellError};

/// Returns the short-format `git status` output for the repository at `repo`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "for the status/staging panel (#24)")
)]
pub async fn status(shell: &impl Shell, repo: &Path) -> Result<String, ShellError> {
    let output = shell.run(repo, "git", &["status", "--short"]).await?;
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{MockShell, ShellOutput};
    use std::env;

    #[tokio::test]
    async fn returns_shell_stdout() {
        let repo = env::current_dir().unwrap();
        let shell = MockShell::new().when(
            "git",
            &["status", "--short"],
            Ok(ShellOutput {
                stdout: " M src/main.rs".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );

        let output = status(&shell, repo.as_path()).await.unwrap();

        assert_eq!(output, " M src/main.rs");
    }
}
