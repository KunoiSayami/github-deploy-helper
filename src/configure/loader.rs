use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use super::types::{
    parse_send_to, FilterMode, TomlCommands, TomlCommitFilter, TomlConfig, TomlProjectOverride,
};

#[derive(Clone)]
pub struct CommitFilter {
    mode: FilterMode,
    globs: Vec<String>,
}

impl CommitFilter {
    pub fn mode(&self) -> FilterMode {
        self.mode.clone()
    }
    pub fn globs(&self) -> &[String] {
        &self.globs
    }
}

impl From<&TomlCommitFilter> for CommitFilter {
    fn from(t: &TomlCommitFilter) -> Self {
        Self {
            mode: t.mode(),
            globs: t.globs().to_vec(),
        }
    }
}

pub struct Commands {
    stop: Option<String>,
    pull: String,
    init: Option<String>,
    update: Option<String>,
    start: Option<String>,
    restart: Option<String>,
}

impl Commands {
    pub fn stop(&self) -> Option<&str> {
        self.stop.as_deref()
    }
    pub fn pull(&self) -> &str {
        &self.pull
    }
    pub fn init(&self) -> Option<&str> {
        self.init.as_deref()
    }
    pub fn update(&self) -> Option<&str> {
        self.update.as_deref()
    }
    pub fn start(&self) -> Option<&str> {
        self.start.as_deref()
    }
    pub fn restart(&self) -> Option<&str> {
        self.restart.as_deref()
    }
}

impl From<&TomlCommands> for Commands {
    fn from(t: &TomlCommands) -> Self {
        Self {
            stop: t.stop().map(str::to_owned),
            pull: t.pull().to_owned(),
            init: t.init().map(str::to_owned),
            update: t.update().map(str::to_owned),
            start: t.start().map(str::to_owned),
            restart: t.restart().map(str::to_owned),
        }
    }
}

pub struct Project {
    name: String,
    http_path: String,
    working_dir: String,
    secret: String,
    branch: String,
    effective_timeout: Duration,
    bypass: bool,
    commit_filter: Option<CommitFilter>,
    commands: Commands,
    pub first_deploy: Arc<AtomicBool>,
}

impl Project {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn http_path(&self) -> &str {
        &self.http_path
    }
    pub fn working_dir(&self) -> &str {
        &self.working_dir
    }
    pub fn secret(&self) -> &str {
        &self.secret
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
    pub fn effective_timeout(&self) -> Duration {
        self.effective_timeout
    }
    pub fn bypass(&self) -> bool {
        self.bypass
    }
    pub fn commit_filter(&self) -> Option<&CommitFilter> {
        self.commit_filter.as_ref()
    }
    pub fn commands(&self) -> &Commands {
        &self.commands
    }
}

pub struct TelegramConfig {
    bot_token: String,
    api_server: Option<String>,
    send_to: Vec<i64>,
}

impl TelegramConfig {
    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }
    pub fn api_server(&self) -> Option<&str> {
        self.api_server.as_deref()
    }
    pub fn send_to(&self) -> &[i64] {
        &self.send_to
    }
}

pub struct Config {
    bind: String,
    log_dir: String,
    #[allow(dead_code)]
    default_timeout: Duration,
    telegram: Option<TelegramConfig>,
    projects: HashMap<String, Arc<Project>>,
}

impl Config {
    pub fn bind(&self) -> &str {
        &self.bind
    }
    pub fn log_dir(&self) -> &str {
        &self.log_dir
    }
    #[allow(dead_code)]
    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }
    pub fn telegram(&self) -> Option<&TelegramConfig> {
        self.telegram.as_ref()
    }
    pub fn projects(&self) -> &HashMap<String, Arc<Project>> {
        &self.projects
    }
}

pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("Cannot read config file: {}", path.as_ref().display()))?;
    let toml: TomlConfig = toml::from_str(&text).context("Failed to parse config.toml")?;

    let default_timeout = Duration::from_secs(toml.default_timeout());

    let telegram = toml.telegram().map(|t| TelegramConfig {
        bot_token: t.bot_token().to_owned(),
        api_server: t.api_server().map(str::to_owned),
        send_to: parse_send_to(t.send_to()),
    });

    let mut projects = HashMap::new();

    for tp in toml.projects() {
        let (branch, timeout_override, bypass, commit_filter, commands) = if tp.deploy_toml() {
            let deploy_path = format!("{}/deploy.toml", tp.working_dir());
            let override_text = std::fs::read_to_string(&deploy_path)
                .with_context(|| format!("Cannot read deploy.toml at {deploy_path}"))?;
            let ov: TomlProjectOverride =
                toml::from_str(&override_text).context("Failed to parse deploy.toml")?;
            let merged_commands = tp
                .commands()
                .merge_from(ov.commands.as_ref().unwrap_or(&TomlCommands::default()));
            let branch = ov.branch.unwrap_or_else(|| tp.branch().to_owned());
            let timeout = ov.timeout.or(tp.timeout());
            let bypass = ov.bypass.unwrap_or(tp.bypass());
            let filter = ov
                .commit_filter
                .as_ref()
                .or(tp.commit_filter())
                .map(CommitFilter::from);
            (branch, timeout, bypass, filter, merged_commands)
        } else {
            (
                tp.branch().to_owned(),
                tp.timeout(),
                tp.bypass(),
                tp.commit_filter().map(CommitFilter::from),
                tp.commands().clone(),
            )
        };

        let effective_timeout =
            Duration::from_secs(timeout_override.unwrap_or(default_timeout.as_secs()));

        let http_path = format!("/webhook/{}", tp.name());

        let project = Arc::new(Project {
            name: tp.name().to_owned(),
            http_path: http_path.clone(),
            working_dir: tp.working_dir().to_owned(),
            secret: tp.secret().to_owned(),
            branch,
            effective_timeout,
            bypass,
            commit_filter,
            commands: Commands::from(&commands),
            first_deploy: Arc::new(AtomicBool::new(true)),
        });

        projects.insert(http_path, project);
    }

    Ok(Config {
        bind: toml.bind().to_owned(),
        log_dir: toml.log_dir().to_owned(),
        default_timeout,
        telegram,
        projects,
    })
}
