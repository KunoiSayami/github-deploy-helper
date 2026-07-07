use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use arc_swap::ArcSwap;
use axum::Router;
use axum::routing::post;
use clap::Parser;
use dashmap::DashMap;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use auth::github_app::GithubAppAuth;
use configure::Project;
use deploy::lock::{DeployLock, DeployLockMap};
use notify::telegram::TelegramNotifier;

mod auth;
mod configure;
mod deploy;
mod logging;
mod notify;
mod state;
mod webhook;

#[derive(Parser)]
#[command(name = "github-deploy-helper", version)]
struct Args {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Allow concurrent deploys per project (disables per-project locking)
    #[arg(long)]
    no_lock: bool,

    /// Re-run init command on the next deploy for all projects
    #[arg(long)]
    force_init: bool,

    /// Increase logging verbosity (repeatable), unmuting noisier dependency
    /// targets (h2, hyper_util, rustls, ...) at each step.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

pub struct AppState {
    pub projects: HashMap<String, Arc<Project>>,
    pub locks: DeployLockMap,
    pub no_lock: bool,
    pub notifier: Option<TelegramNotifier>,
    pub log_dir: PathBuf,
    pub github_app: Option<Arc<GithubAppAuth>>,
    pub state: Arc<state::SharedState>,
    pub shell: String,
}

/// Reloadable handle to the current `AppState` snapshot. Request handlers call
/// `load_full()` to get a consistent `Arc<AppState>` for the duration of one
/// request; `configure::watcher` swaps in a new snapshot on config changes.
pub type SharedAppState = Arc<ArcSwap<AppState>>;

/// Builds the EnvFilter used for logging: `default_level` when `RUST_LOG` is unset,
/// with noisy dependency targets progressively unmuted as `verbose` increases.
fn build_env_filter(verbose: u8, default_level: &str) -> EnvFilter {
    let mut filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    if verbose < 5 {
        filter = filter.add_directive("quinn_proto::connection=warn".parse().unwrap());
    }
    if verbose < 4 {
        filter = filter
            .add_directive("h2::proto=warn".parse().unwrap())
            .add_directive("rustls::client=warn".parse().unwrap())
            .add_directive("quinn_proto=warn".parse().unwrap())
            .add_directive("rustls_platform_verifier=warn".parse().unwrap());
    }
    if verbose < 3 {
        filter = filter
            .add_directive("h2::codec=warn".parse().unwrap())
            .add_directive("h2::hpack=warn".parse().unwrap())
            .add_directive("h2::client=warn".parse().unwrap());
    }
    if verbose < 2 {
        filter = filter
            .add_directive("hyper_util::client=warn".parse().unwrap())
            .add_directive("hickory_proto=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("h2::frame=warn".parse().unwrap());
    }
    if verbose < 1 {
        filter = filter.add_directive("reqwest::connect=warn".parse().unwrap());
    }

    filter
}

fn init_tracing(log_dir: &Path, verbose: u8) -> Vec<WorkerGuard> {
    use tracing_subscriber::prelude::*;

    let mut guards = Vec::new();

    let env_filter = build_env_filter(verbose, "github_deploy_helper=info");

    // journald already timestamps each line, so drop tracing's own timestamp under systemd
    let under_systemd = std::env::var_os("JOURNAL_STREAM").is_some();
    let console_layer = if under_systemd {
        tracing_subscriber::fmt::layer().without_time().boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    }
    .with_filter(env_filter);

    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("Warning: cannot create log dir: {e}");
    }

    let appender = tracing_appender::rolling::daily(log_dir, "deploy.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    guards.push(guard);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    guards
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Multiple dependencies (reqwest, teloxide) link in both the aws-lc-rs and ring
    // rustls crypto backends, so no default provider gets installed automatically.
    // Pin one explicitly before any TLS handshake or JWT signing happens.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("failed to install rustls CryptoProvider");

    let args = Args::parse();

    let config = configure::load(&args.config)?;

    let log_dir = PathBuf::from(config.log_dir());
    let log_keep_days = config.log_keep_days();
    let _guards = init_tracing(&log_dir, args.verbose);

    logging::cleaner::clean_old_logs(&log_dir, log_keep_days);
    tokio::spawn(logging::cleaner::start_cleaner(
        log_dir.clone(),
        log_keep_days,
    ));

    info!("Loaded {} project(s)", config.projects().len());

    let shared_state = Arc::new(state::SharedState::load(config.state_file())?);

    let notifier = config.telegram().map(|tg| {
        notify::telegram::start(
            tg.bot_token().to_owned(),
            tg.api_server().map(str::to_owned),
            tg.send_to().to_vec(),
        )
    });

    let github_app = config
        .github_app()
        .map(|g| {
            let private_key = std::fs::read_to_string(g.private_key_path()).with_context(|| {
                format!(
                    "Cannot read GitHub App private key at {}",
                    g.private_key_path()
                )
            })?;
            GithubAppAuth::new(g.app_id(), &private_key).map(Arc::new)
        })
        .transpose()?;

    configure::bootstrap::clone_missing_projects(config.projects(), github_app.as_deref()).await;
    configure::bootstrap::ensure_webhooks(
        config.projects(),
        github_app.as_deref(),
        config.github_app(),
    )
    .await;

    if args.force_init {
        for project in config.projects().values() {
            if let Err(e) = shared_state.clear_initialized(project.name()).await {
                tracing::warn!(project = project.name(), error = %e, "failed to persist force_init state");
            }
        }
        info!("force_init: all projects will re-run init on next deploy");
    }

    let locks: DeployLockMap = Arc::new(DashMap::new());
    for project in config.projects().values() {
        locks.insert(
            project.http_path().to_owned(),
            Arc::new(DeployLock::default()),
        );
        info!(
            project = project.name(),
            path = project.http_path(),
            "registered project"
        );
    }

    let bind = config.bind().to_owned();

    let app_state: SharedAppState = Arc::new(ArcSwap::from_pointee(AppState {
        projects: config.projects().clone(),
        locks,
        no_lock: args.no_lock,
        notifier,
        github_app,
        log_dir,
        state: shared_state,
        shell: config.shell().to_owned(),
    }));

    tokio::spawn(configure::watcher::watch(
        args.config.clone(),
        app_state.clone(),
        config,
    ));

    let router = Router::new()
        .route("/webhook/{name}", post(webhook::handler::handle))
        .with_state(app_state.clone());

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
