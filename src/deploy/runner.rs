use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Runs a shell command in `working_dir` with a `timeout`.
/// The command string is passed to `sh -c`. `extra_env`, if set, is passed via the
/// process environment rather than interpolated into the command string, so secrets
/// (e.g. a GitHub App installation token) never appear in argv/process listings.
pub async fn run(
    cmd: &str,
    working_dir: &str,
    timeout: Duration,
    extra_env: Option<(&str, &str)>,
) -> anyhow::Result<CommandOutput> {
    let mut command = Command::new("sh");
    command
        .args(["-c", cmd])
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some((key, value)) = extra_env {
        command.env(key, value);
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
