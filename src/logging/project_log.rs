use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::deploy::runner::CommandOutput;

const SECS_PER_DAY: u64 = 86_400;

/// Appends the raw output of a single deploy step to
/// `<log_dir>/<project>/<YYYY-MM-DD>.log`, kept separate from the shared `deploy.log`
/// tracing output so command stdout/stderr isn't truncated or interleaved with
/// unrelated application log lines. Cleanup of old files is handled by
/// `logging::cleaner`, which recurses into per-project folders.
pub struct ProjectLog {
    dir: PathBuf,
}

impl ProjectLog {
    pub fn new(log_dir: &Path, project: &str) -> Self {
        Self {
            dir: log_dir.join(project),
        }
    }

    fn today_path(&self) -> PathBuf {
        self.dir.join(format!("{}.log", today_date_string()))
    }

    pub async fn write_command(&self, step: &str, cmd: &str, out: &CommandOutput) {
        if let Err(e) = tokio::fs::create_dir_all(&self.dir).await {
            tracing::warn!(dir = %self.dir.display(), error = %e, "failed to create project log dir");
            return;
        }

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

        let path = self.today_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await;
        match file {
            Ok(mut f) => {
                if let Err(e) = f.write_all(body.as_bytes()).await {
                    tracing::warn!(path = %path.display(), error = %e, "failed to write project log");
                }
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to open project log");
            }
        }
    }
}

/// Returns today's UTC date as `YYYY-MM-DD`, without pulling in a date/time crate.
fn today_date_string() -> String {
    let epoch_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days_since_epoch = epoch_secs / SECS_PER_DAY;
    let (y, m, d) = civil_from_days(days_since_epoch as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Converts a day count since the Unix epoch (1970-01-01) to a proleptic Gregorian
/// (year, month, day), using Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_458), (2026, 1, 5));
    }

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

        let path = dir
            .join("my-project")
            .join(format!("{}.log", today_date_string()));
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("step=pull status=ok"));
        assert!(contents.contains("fetched 3 objects"));
        assert!(contents.contains("step=start status=failed"));
        assert!(contents.contains("-- stderr --"));
        assert!(contents.contains("unit not found"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
