use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::routing::post;
use axum::Router;
use clap::Parser;
use dashmap::DashMap;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use configure::Project;
use deploy::lock::{DeployLock, DeployLockMap};
use notify::telegram::TelegramNotifier;

mod configure;
mod deploy;
mod logging;
mod notify;
mod webhook;

#[derive(Parser)]
#[command(name = "github-deploy-helper", version)]
struct Args {
    /// Path to config file
    #[arg(short, long, default_value = "data/config.toml")]
    config: PathBuf,

    /// Allow concurrent deploys per project (disables per-project locking)
    #[arg(long)]
    no_lock: bool,

    /// Re-run init command on the next deploy for all projects
    #[arg(long)]
    force_init: bool,
}

pub struct AppState {
    pub projects: HashMap<String, Arc<Project>>,
    pub locks: DeployLockMap,
    pub no_lock: bool,
    pub notifier: Option<TelegramNotifier>,
    pub log_dir: PathBuf,
}

fn init_tracing(log_dir: &Path, projects: &[&str]) -> Vec<WorkerGuard> {
    use tracing_subscriber::prelude::*;

    let mut guards = Vec::new();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("github_deploy_helper=info"));

    let console_layer = tracing_subscriber::fmt::layer().with_filter(env_filter);

    // Combined non-blocking writer: all project execution logs go to a single
    // file (deploy.log) with project=name fields for filtering. One file keeps
    // the tracing subscriber setup simple.
    let combined_log = log_dir.join("deploy.log");
    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("Warning: cannot create log dir: {e}");
    }

    let _ = projects; // per-project writers replaced by single combined log

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&combined_log)
    {
        Ok(_) => {
            let appender = tracing_appender::rolling::never(log_dir, "deploy.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            guards.push(guard);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false);
            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
        }
        Err(e) => {
            eprintln!("Warning: cannot open deploy.log: {e}");
            tracing_subscriber::registry().with(console_layer).init();
        }
    }

    guards
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let config = configure::load(&args.config)?;

    let project_names: Vec<&str> = config.projects().values().map(|p| p.name()).collect();
    let log_dir = PathBuf::from(config.log_dir());
    let _guards = init_tracing(&log_dir, &project_names);

    info!("Loaded {} project(s)", config.projects().len());

    let notifier = config.telegram().map(|tg| {
        notify::telegram::start(
            tg.bot_token().to_owned(),
            tg.api_server().map(str::to_owned),
            tg.send_to().to_vec(),
        )
    });

    if args.force_init {
        for project in config.projects().values() {
            project
                .first_deploy
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        info!("force_init: all projects will re-run init on next deploy");
    }

    let locks: DeployLockMap = Arc::new(DashMap::new());
    for path in config.projects().keys() {
        locks.insert(path.clone(), Arc::new(DeployLock::default()));
    }

    let state = Arc::new(AppState {
        projects: config.projects().clone(),
        locks,
        no_lock: args.no_lock,
        notifier,
        log_dir,
    });

    let mut router = Router::new();
    for (path, project) in config.projects() {
        let project_path = path.clone();
        let state_clone = state.clone();

        // Inject project path via a custom header so the generic handler can look it up
        let handler_state = state_clone.clone();
        router = router.route(
            path,
            post({
                let path_header = project_path.clone();
                move |mut headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let state = handler_state.clone();
                    headers.insert("X-Original-Path", path_header.parse().unwrap());
                    async move {
                        webhook::handler::handle(axum::extract::State(state), headers, body).await
                    }
                }
            }),
        );

        info!(
            project = project.name(),
            path = project_path,
            "registered webhook route"
        );
    }

    let bind = config.bind().to_owned();
    info!("Listening on {bind}");

    let listener = tokio::net::TcpListener::bind(&bind).await?;

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutting down");
        })
        .await?;

    Ok(())
}
