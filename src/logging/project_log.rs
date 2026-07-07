use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::deploy::runner::CommandOutput;

/// Appends the raw output of a single deploy step to `<log_dir>/<project>.log`, kept
/// separate from the shared `deploy.log` tracing output so command stdout/stderr isn't
/// truncated or interleaved with unrelated application log lines.
pub struct ProjectLog {
    path: std::path::PathBuf,
}

impl ProjectLog {
    pub fn new(log_dir: &Path, project: &str) -> Self {
        Self {
            path: log_dir.join(format!("{project}.log")),
        }
    }

    pub async fn write_command(&self, step: &str, cmd: &str, out: &CommandOutput) {
        let status = if out.success { "ok" } else { "failed" };
        let epoch_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut body = format!("==== unix={epoch_secs} step={step} status={status}\n$ {cmd}\n");
        if !out.stdout.is_empty() {
            body.push_str(&out.stdout);
            if !out.stdout.ends_with('\n') {
                body.push('\n');
            }
        }
        if !out.stderr.is_empty() {
            body.push_str("-- stderr --\n");
            body.push_str(&out.stderr);
            if !out.stderr.ends_with('\n') {
                body.push('\n');
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await;
        match file {
            Ok(mut f) => {
                if let Err(e) = f.write_all(body.as_bytes()).await {
                    tracing::warn!(path = %self.path.display(), error = %e, "failed to write project log");
                }
            }
            Err(e) => {
                tracing::warn!(path = %self.path.display(), error = %e, "failed to open project log");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_stdout_and_stderr_to_project_file() {
        let dir = std::env::temp_dir().join(format!("glh-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let log = ProjectLog::new(&dir, "my-project");
        log.write_command(
            "pull",
            "git fetch",
            &CommandOutput {
                stdout: "fetched 3 objects".to_owned(),
                stderr: String::new(),
                success: true,
            },
        )
        .await;
        log.write_command(
            "start",
            "systemctl start x",
            &CommandOutput {
                stdout: String::new(),
                stderr: "unit not found".to_owned(),
                success: false,
            },
        )
        .await;

        let path = dir.join("my-project.log");
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("step=pull status=ok"));
        assert!(contents.contains("fetched 3 objects"));
        assert!(contents.contains("step=start status=failed"));
        assert!(contents.contains("-- stderr --"));
        assert!(contents.contains("unit not found"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
