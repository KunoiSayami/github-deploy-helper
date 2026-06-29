use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Runs a shell command in `working_dir` with a `timeout`.
/// The command string is passed to `sh -c`.
pub async fn run(cmd: &str, working_dir: &str, timeout: Duration) -> anyhow::Result<CommandOutput> {
    let child = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
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
