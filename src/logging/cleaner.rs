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

    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("log cleaner: cannot read log dir: {e}");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
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
