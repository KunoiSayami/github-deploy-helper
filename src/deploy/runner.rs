use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Context;
use regex::Regex;
use tokio::process::Command;

static ANSI_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b(\[[0-9;]*[A-Za-z]|\([A-Za-z0-9])").unwrap());

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl CommandOutput {
    /// Builds a short failure summary for notifications: the last few lines of
    /// stderr, falling back to stdout if stderr is empty. ANSI escape codes
    /// (color, cursor movement) are stripped since notifications are plain text.
    pub fn failure_summary(&self, name: &str) -> String {
        const MAX_LINES: usize = 8;

        let source = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let source = ANSI_ESCAPE.replace_all(source, "");
        let tail: Vec<&str> = source.lines().rev().take(MAX_LINES).collect();

        if tail.is_empty() {
            return format!("exit failure, see {name}/ log folder");
        }
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        format!("exit failure:\n{}", tail.join("\n"))
    }
}

/// Runs a shell command in `working_dir` with a `timeout`.
/// The command string is passed to `<shell> -c`. `extra_env`, if set, is passed via the
/// process environment rather than interpolated into the command string, so secrets
/// (e.g. a GitHub App installation token) never appear in argv/process listings.
/// `extra_git_config`, if set, is exposed via `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/
/// `GIT_CONFIG_VALUE_n`, which any `git` invocation inside `cmd` picks up automatically
/// (equivalent to passing `-c <key>=<value>` to every git subcommand, without the caller
/// having to hand-write that into their pull command).
pub async fn run(
    cmd: &str,
    working_dir: &str,
    timeout: Duration,
    extra_env: Option<(&str, &str)>,
    extra_git_config: Option<(&str, &str)>,
    shell: &str,
) -> anyhow::Result<CommandOutput> {
    let mut command = Command::new(shell);
    command
        .args(["-c", cmd])
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some((key, value)) = extra_env {
        command.env(key, value);
    }
    if let Some((key, value)) = extra_git_config {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", key)
            .env("GIT_CONFIG_VALUE_0", value);
    }
    let child = command
        .spawn()
        .with_context(|| format!("Failed to spawn command: {cmd}"))?;

    let result = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .with_context(|| format!("Command timed out after {}s: {cmd}", timeout.as_secs()))??;

    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        success: result.status.success(),
    })
}
