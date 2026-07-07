use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

/// Builds the `http.extraHeader` config value for a GitHub App installation token,
/// suitable for `git -c http.extraHeader=<value>` or the `GIT_CONFIG_VALUE_n` env var.
pub fn basic_auth_header_value(token: &str) -> String {
    let basic = BASE64.encode(format!("x-access-token:{token}"));
    format!("AUTHORIZATION: basic {basic}")
}

/// Clones `git_url` into `working_dir` at `branch`. If `gh_token` is set, it is sent as
/// a GitHub App installation token via `http.extraHeader` rather than embedded in the
/// URL or passed as a plain argv argument, so it never appears in process listings.
pub async fn clone(
    git_url: &str,
    working_dir: &str,
    branch: &str,
    gh_token: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(parent) = std::path::Path::new(working_dir).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut command = tokio::process::Command::new("git");

    if let Some(token) = gh_token {
        command.args([
            "-c",
            &format!("http.extraHeader={}", basic_auth_header_value(token)),
        ]);
    }

    let out = command
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
