use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

use super::types::{
    FilterMode, TomlAuthMode, TomlCommands, TomlCommitFilter, TomlCommitVerify, TomlConfig,
    TomlProjectAuth, TomlProjectOverride, parse_send_to,
};

#[derive(Clone)]
pub struct CommitFilter {
    mode: FilterMode,
    globs: Vec<String>,
    message_patterns: Vec<String>,
}

impl CommitFilter {
    pub fn mode(&self) -> FilterMode {
        self.mode.clone()
    }
    pub fn globs(&self) -> &[String] {
        &self.globs
    }
    pub fn message_patterns(&self) -> &[String] {
        &self.message_patterns
    }
}

impl From<&TomlCommitFilter> for CommitFilter {
    fn from(t: &TomlCommitFilter) -> Self {
        Self {
            mode: t.mode(),
            globs: t.globs().to_vec(),
            message_patterns: t.message_patterns().to_vec(),
        }
    }
}

#[derive(Clone, Default)]
pub struct CommitVerify {
    allowed_authors: Vec<String>,
    require_signed: bool,
}

impl CommitVerify {
    pub fn allowed_authors(&self) -> &[String] {
        &self.allowed_authors
    }
    pub fn require_signed(&self) -> bool {
        self.require_signed
    }
    #[cfg(test)]
    pub fn for_test(allowed_authors: Vec<String>, require_signed: bool) -> Self {
        Self {
            allowed_authors: allowed_authors.iter().map(|a| a.to_lowercase()).collect(),
            require_signed,
        }
    }
}

impl From<&TomlCommitVerify> for CommitVerify {
    fn from(t: &TomlCommitVerify) -> Self {
        Self {
            allowed_authors: t
                .allowed_authors()
                .iter()
                .map(|a| a.to_lowercase())
                .collect(),
            require_signed: t.require_signed(),
        }
    }
}

pub struct Commands {
    stop: Option<String>,
    pull: Option<String>,
    init: Option<String>,
    update: Option<String>,
    start: Option<String>,
    restart: Option<String>,
}

impl Commands {
    pub fn stop(&self) -> Option<&str> {
        self.stop.as_deref()
    }
    pub fn pull(&self) -> Option<&str> {
        self.pull.as_deref()
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
            stop: t.stop(),
            pull: t.pull(),
            init: t.init(),
            update: t.update(),
            start: t.start(),
            restart: t.restart(),
        }
    }
}

#[derive(Clone)]
pub enum ProjectAuth {
    Ssh,
    GithubApp { owner: String, repo: String },
}

impl TryFrom<&TomlProjectAuth> for ProjectAuth {
    type Error = anyhow::Error;

    fn try_from(t: &TomlProjectAuth) -> anyhow::Result<Self> {
        match t.mode() {
            TomlAuthMode::Ssh => Ok(Self::Ssh),
            TomlAuthMode::GithubApp => {
                let owner = t
                    .owner()
                    .context("auth.mode = \"github_app\" requires auth.owner")?
                    .to_owned();
                let repo = t
                    .repo()
                    .context("auth.mode = \"github_app\" requires auth.repo")?
                    .to_owned();
                Ok(Self::GithubApp { owner, repo })
            }
        }
    }
}

pub struct GithubAppConfig {
    app_id: u64,
    private_key_path: String,
    public_base_url: Option<String>,
}

impl GithubAppConfig {
    pub fn app_id(&self) -> u64 {
        self.app_id
    }
    pub fn private_key_path(&self) -> &str {
        &self.private_key_path
    }
    pub fn public_base_url(&self) -> Option<&str> {
        self.public_base_url.as_deref()
    }
}

pub struct Project {
    name: String,
    http_path: String,
    working_dir: String,
    git_url: Option<String>,
    secret: String,
    branch: String,
    effective_timeout: Duration,
    bypass: bool,
    commit_filter: Option<CommitFilter>,
    commit_verify: Option<CommitVerify>,
    commands: Commands,
    auth: ProjectAuth,
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
    /// Clone URL used to bootstrap `working_dir` if it doesn't exist yet.
    pub fn git_url(&self) -> Option<&str> {
        self.git_url.as_deref()
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
    pub fn commit_verify(&self) -> Option<&CommitVerify> {
        self.commit_verify.as_ref()
    }
    pub fn commands(&self) -> &Commands {
        &self.commands
    }
    pub fn auth(&self) -> &ProjectAuth {
        &self.auth
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
    log_keep_days: u64,
    state_file: String,
    shell: String,
    telegram: Option<TelegramConfig>,
    github_app: Option<GithubAppConfig>,
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
    pub fn log_keep_days(&self) -> u64 {
        self.log_keep_days
    }
    pub fn state_file(&self) -> &str {
        &self.state_file
    }
    pub fn shell(&self) -> &str {
        &self.shell
    }
    pub fn telegram(&self) -> Option<&TelegramConfig> {
        self.telegram.as_ref()
    }
    pub fn github_app(&self) -> Option<&GithubAppConfig> {
        self.github_app.as_ref()
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

    let github_app = toml.github_app().map(|g| GithubAppConfig {
        app_id: g.app_id(),
        private_key_path: g.private_key_path().to_owned(),
        public_base_url: g.public_base_url().map(str::to_owned),
    });

    let mut projects = HashMap::new();

    for tp in toml.projects() {
        let (branch, timeout_override, bypass, commit_filter, commit_verify, commands, auth) =
            if tp.deploy_toml() {
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
                let verify = ov
                    .commit_verify
                    .as_ref()
                    .or(tp.commit_verify())
                    .map(CommitVerify::from);
                let auth = ov.auth.clone().or_else(|| tp.auth().cloned());
                (
                    branch,
                    timeout,
                    bypass,
                    filter,
                    verify,
                    merged_commands,
                    auth,
                )
            } else {
                (
                    tp.branch().to_owned(),
                    tp.timeout(),
                    tp.bypass(),
                    tp.commit_filter().map(CommitFilter::from),
                    tp.commit_verify().map(CommitVerify::from),
                    tp.commands().clone(),
                    tp.auth().cloned(),
                )
            };

        let auth = match &auth {
            Some(a) => ProjectAuth::try_from(a)
                .with_context(|| format!("Invalid auth config for project {}", tp.name()))?,
            None => ProjectAuth::Ssh,
        };
        if matches!(auth, ProjectAuth::GithubApp { .. }) && github_app.is_none() {
            anyhow::bail!(
                "Project {} uses auth.mode = \"github_app\" but no [github_app] config is present",
                tp.name()
            );
        }

        let effective_timeout =
            Duration::from_secs(timeout_override.unwrap_or(default_timeout.as_secs()));

        let http_path = format!("/webhook/{}", tp.name());

        let project = Arc::new(Project {
            name: tp.name().to_owned(),
            http_path: http_path.clone(),
            working_dir: tp.working_dir().to_owned(),
            git_url: tp.git_url().map(str::to_owned),
            secret: tp.secret().to_owned(),
            branch,
            effective_timeout,
            bypass,
            commit_filter,
            commit_verify,
            commands: Commands::from(&commands),
            auth,
        });

        projects.insert(http_path, project);
    }

    Ok(Config {
        bind: toml.bind().to_owned(),
        log_dir: toml.log_dir().to_owned(),
        default_timeout,
        log_keep_days: toml.log_keep_days(),
        state_file: toml.state_file().to_owned(),
        shell: toml.shell().to_owned(),
        telegram,
        github_app,
        projects,
    })
}
