use serde::Deserialize;
use toml::Value;

#[derive(Deserialize, Clone)]
pub struct TomlConfig {
    bind: String,
    log_dir: String,
    default_timeout: Option<u64>,
    telegram: Option<TomlTelegram>,
    projects: Vec<TomlProject>,
}

impl TomlConfig {
    pub fn bind(&self) -> &str {
        &self.bind
    }
    pub fn log_dir(&self) -> &str {
        &self.log_dir
    }
    pub fn default_timeout(&self) -> u64 {
        self.default_timeout.unwrap_or(30)
    }
    pub fn telegram(&self) -> Option<&TomlTelegram> {
        self.telegram.as_ref()
    }
    pub fn projects(&self) -> &[TomlProject] {
        &self.projects
    }
}

#[derive(Deserialize, Clone)]
pub struct TomlTelegram {
    bot_token: String,
    api_server: Option<String>,
    send_to: Value,
}

impl TomlTelegram {
    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }
    pub fn api_server(&self) -> Option<&str> {
        self.api_server.as_deref()
    }
    pub fn send_to(&self) -> &Value {
        &self.send_to
    }
}

#[derive(Deserialize, Clone)]
pub struct TomlProject {
    name: String,
    working_dir: String,
    secret: String,
    branch: String,
    timeout: Option<u64>,
    bypass: Option<bool>,
    deploy_toml: Option<bool>,
    commit_filter: Option<TomlCommitFilter>,
    commands: TomlCommands,
}

impl TomlProject {
    pub fn name(&self) -> &str {
        &self.name
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
    pub fn timeout(&self) -> Option<u64> {
        self.timeout
    }
    pub fn bypass(&self) -> bool {
        self.bypass.unwrap_or(false)
    }
    pub fn deploy_toml(&self) -> bool {
        self.deploy_toml.unwrap_or(false)
    }
    pub fn commit_filter(&self) -> Option<&TomlCommitFilter> {
        self.commit_filter.as_ref()
    }
    pub fn commands(&self) -> &TomlCommands {
        &self.commands
    }
}

#[derive(Deserialize, Clone, Default)]
pub struct TomlProjectOverride {
    pub branch: Option<String>,
    pub timeout: Option<u64>,
    pub bypass: Option<bool>,
    pub commit_filter: Option<TomlCommitFilter>,
    pub commands: Option<TomlCommands>,
}

#[derive(Deserialize, Clone)]
pub struct TomlCommitFilter {
    mode: FilterMode,
    globs: Vec<String>,
}

impl TomlCommitFilter {
    pub fn mode(&self) -> FilterMode {
        self.mode.clone()
    }
    pub fn globs(&self) -> &[String] {
        &self.globs
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    Include,
    Exclude,
}

#[derive(Deserialize, Clone, Default)]
pub struct TomlCommands {
    stop: Option<String>,
    pull: Option<String>,
    init: Option<String>,
    update: Option<String>,
    start: Option<String>,
    restart: Option<String>,
}

impl TomlCommands {
    pub fn stop(&self) -> Option<&str> {
        self.stop.as_deref()
    }
    pub fn pull(&self) -> &str {
        self.pull.as_deref().unwrap_or("git pull --ff-only")
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
    pub fn merge_from(&self, other: &TomlCommands) -> TomlCommands {
        TomlCommands {
            stop: other.stop.clone().or_else(|| self.stop.clone()),
            pull: other.pull.clone().or_else(|| self.pull.clone()),
            init: other.init.clone().or_else(|| self.init.clone()),
            update: other.update.clone().or_else(|| self.update.clone()),
            start: other.start.clone().or_else(|| self.start.clone()),
            restart: other.restart.clone().or_else(|| self.restart.clone()),
        }
    }
}

pub fn parse_send_to(value: &Value) -> Vec<i64> {
    match value {
        Value::String(s) => vec![s.parse().expect("Cannot parse send_to string as i64")],
        Value::Integer(i) => vec![*i],
        Value::Array(arr) => arr
            .iter()
            .map(|v| match v {
                Value::String(s) => s.parse().expect("Cannot parse send_to array string as i64"),
                Value::Integer(i) => *i,
                _ => panic!("Unexpected send_to array element: {v:?}"),
            })
            .collect(),
        _ => panic!("Unexpected send_to value: {value:?}"),
    }
}
