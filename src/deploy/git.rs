pub async fn clone(git_url: &str, working_dir: &str, branch: &str) -> anyhow::Result<()> {
    if let Some(parent) = std::path::Path::new(working_dir).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let out = tokio::process::Command::new("git")
        .args(["clone", "--branch", branch, git_url, working_dir])
        .output()
        .await?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("git clone failed: {stderr}");
    }

    tracing::info!(url = git_url, path = working_dir, "git clone succeeded");
    Ok(())
}
