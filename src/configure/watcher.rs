use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::auth::github_app::GithubAppAuth;
use crate::notify::telegram;
use crate::{AppState, SharedAppState};

use super::bootstrap;
use super::loader::Config;

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Watches `config_path` and every project's `working_dir/deploy.toml` (if
/// present) and hot-reloads `app_state` when one of them changes. Runs until
/// the process exits; reload failures are logged and the last-good config
/// keeps serving.
///
/// Watches are placed on individual files rather than recursively on
/// `config_path`'s directory: `log_dir` or a project's `working_dir` often
/// live alongside `config.toml`, and recursively watching would pick up
/// unrelated churn (log rotation, git operations) as spurious reload
/// triggers.
///
/// `last_config` is the config loaded at startup, kept around purely to diff
/// against on the first reload (Telegram/GitHub App instances aren't
/// introspectable, so we can't tell if they changed just from `AppState`).
pub async fn watch(config_path: PathBuf, app_state: SharedAppState, last_config: Config) {
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();

    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| match res
    {
        Ok(event) if is_relevant(&event.kind) => {
            let _ = tx.send(());
        }
        Ok(_) => {}
        Err(e) => error!(error = %e, "config watcher error"),
    }) {
        Ok(w) => w,
        Err(e) => {
            error!(error = %e, "failed to start config watcher, hot-reload disabled");
            return;
        }
    };

    if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
        error!(error = %e, path = %config_path.display(), "failed to watch config file, hot-reload disabled");
        return;
    }
    for deploy_toml in last_config
        .projects()
        .values()
        .map(|p| PathBuf::from(p.working_dir()).join("deploy.toml"))
    {
        // Best-effort: a project's deploy.toml may not exist yet, or its
        // working_dir may not be cloned yet, so ignore errors here.
        let _ = watcher.watch(&deploy_toml, RecursiveMode::NonRecursive);
    }
    info!(path = %config_path.display(), "watching for config changes");

    let mut last_config = last_config;
    while rx.recv().await.is_some() {
        // Coalesce a burst of editor-generated events (write + rename + ...)
        // into a single reload, resetting the debounce timer on each event.
        loop {
            match tokio::time::timeout(DEBOUNCE, rx.recv()).await {
                Ok(Some(())) => continue,
                Ok(None) => return,
                Err(_) => break,
            }
        }

        if let Some(reloaded) = reload(&config_path, &app_state, &last_config).await {
            // deploy.toml watches may need to change if projects were
            // added/removed or deploy_toml was toggled; re-derive them.
            for deploy_toml in reloaded
                .projects()
                .values()
                .map(|p| PathBuf::from(p.working_dir()).join("deploy.toml"))
            {
                let _ = watcher.watch(&deploy_toml, RecursiveMode::NonRecursive);
            }
            last_config = reloaded;
        }
    }
}

fn is_relevant(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

/// Reloads config from disk and swaps it into `app_state` if it parses
/// successfully. Returns the newly loaded `Config` (to become the next
/// `last_config`) on success, or `None` if the reload was aborted and the
/// previous config/state should keep serving.
async fn reload(
    config_path: &PathBuf,
    app_state: &SharedAppState,
    last_config: &Config,
) -> Option<Config> {
    let new_config = match super::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            error!(error = ?e, "config reload failed, keeping last-good config");
            return None;
        }
    };

    let current = app_state.load_full();

    let github_app: Option<Arc<GithubAppAuth>> = match new_config.github_app() {
        None => None,
        Some(g) if last_config.github_app() == Some(g) => current.github_app.clone(),
        Some(g) => {
            let private_key = match std::fs::read_to_string(g.private_key_path()) {
                Ok(k) => k,
                Err(e) => {
                    error!(error = %e, path = g.private_key_path(), "config reload: cannot read GitHub App private key, keeping last-good config");
                    return None;
                }
            };
            match GithubAppAuth::new(g.app_id(), &private_key) {
                Ok(auth) => Some(Arc::new(auth)),
                Err(e) => {
                    error!(error = %e, "config reload: invalid GitHub App private key, keeping last-good config");
                    return None;
                }
            }
        }
    };

    bootstrap::clone_missing_projects(new_config.projects(), github_app.as_deref()).await;
    bootstrap::ensure_webhooks(
        new_config.projects(),
        github_app.as_deref(),
        new_config.github_app(),
    )
    .await;

    for name in new_config.projects().keys() {
        if !current.projects.contains_key(name) {
            info!(project = name, "config reload: project added");
        }
    }
    for name in current.projects.keys() {
        if !new_config.projects().contains_key(name) {
            info!(project = name, "config reload: project removed");
        }
    }

    let notifier = match new_config.telegram() {
        None => {
            if let Some(old) = &current.notifier {
                old.shutdown().await;
            }
            None
        }
        Some(tg) if last_config.telegram() == Some(tg) => current.notifier.clone(),
        Some(tg) => {
            if let Some(old) = &current.notifier {
                old.shutdown().await;
            }
            Some(telegram::start(
                tg.bot_token().to_owned(),
                tg.api_server().map(str::to_owned),
                tg.send_to().to_vec(),
            ))
        }
    };

    let new_state = AppState {
        projects: new_config.projects().clone(),
        locks: current.locks.clone(),
        no_lock: current.no_lock,
        notifier,
        github_app,
        log_dir: current.log_dir.clone(),
        state: current.state.clone(),
        shell: new_config.shell().to_owned(),
    };

    app_state.store(Arc::new(new_state));
    info!("config reloaded");
    Some(new_config)
}
