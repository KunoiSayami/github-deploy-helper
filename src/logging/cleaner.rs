use std::path::Path;
use std::time::{Duration, SystemTime};

use tracing::{info, warn};

pub fn clean_old_logs(log_dir: &Path, keep_days: u64) {
    if keep_days == 0 {
        return;
    }

    let cutoff = match SystemTime::now().checked_sub(Duration::from_secs(keep_days * 86400)) {
        Some(t) => t,
        None => return,
    };

    // One level deep: `<log_dir>/deploy.log.<date>` lives directly under `log_dir`,
    // while per-project command logs live under `<log_dir>/<project>/<date>.log`.
    clean_dir(log_dir, cutoff, true);
}

fn clean_dir(dir: &Path, cutoff: SystemTime, recurse: bool) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("log cleaner: cannot read log dir: {e}");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recurse {
                clean_dir(&path, cutoff, false);
            }
            continue;
        }
        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if mtime < cutoff {
            match std::fs::remove_file(&path) {
                Ok(()) => info!(path = %path.display(), "log cleaner: removed old log file"),
                Err(e) => warn!(path = %path.display(), "log cleaner: failed to remove: {e}"),
            }
        }
    }
}

pub async fn start_cleaner(log_dir: std::path::PathBuf, keep_days: u64) {
    if keep_days == 0 {
        return;
    }
    loop {
        clean_old_logs(&log_dir, keep_days);
        tokio::time::sleep(Duration::from_secs(86400)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_mtime(path: &Path, age: Duration) {
        let mtime = SystemTime::now() - age;
        let file = std::fs::File::open(path).unwrap();
        file.set_modified(mtime).unwrap();
    }

    #[test]
    fn removes_old_files_in_project_subfolders_but_not_recent_ones() {
        let dir = std::env::temp_dir().join(format!("glh-test-cleaner-{}", std::process::id()));
        let project_dir = dir.join("my-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let old_top = dir.join("deploy.log.2020-01-01");
        let recent_top = dir.join("deploy.log.today");
        let old_nested = project_dir.join("2020-01-01.log");
        let recent_nested = project_dir.join("today.log");

        for p in [&old_top, &recent_top, &old_nested, &recent_nested] {
            std::fs::write(p, b"x").unwrap();
        }
        set_mtime(&old_top, Duration::from_secs(60 * 86400));
        set_mtime(&old_nested, Duration::from_secs(60 * 86400));

        clean_old_logs(&dir, 30);

        assert!(!old_top.exists());
        assert!(recent_top.exists());
        assert!(!old_nested.exists());
        assert!(recent_nested.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
