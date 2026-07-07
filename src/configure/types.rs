use serde::Deserialize;
use toml::Value;

#[derive(Deserialize, Clone)]
pub struct TomlConfig {
    bind: String,
    log_dir: String,
    default_timeout: Option<u64>,
    log_keep_days: Option<u64>,
    state_file: Option<String>,
    shell: Option<String>,
    telegram: Option<TomlTelegram>,
    github_app: Option<TomlGithubApp>,
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
    pub fn log_keep_days(&self) -> u64 {
        self.log_keep_days.unwrap_or(30)
    }
    pub fn state_file(&self) -> &str {
        self.state_file.as_deref().unwrap_or("state.json")
    }
    pub fn shell(&self) -> &str {
        self.shell.as_deref().unwrap_or("sh")
    }
    pub fn telegram(&self) -> Option<&TomlTelegram> {
        self.telegram.as_ref()
    }
    pub fn github_app(&self) -> Option<&TomlGithubApp> {
        self.github_app.as_ref()
    }
    pub fn projects(&self) -> &[TomlProject] {
        &self.projects
    }
}

#[derive(Deserialize, Clone)]
pub struct TomlGithubApp {
    app_id: u64,
    private_key_path: String,
    public_base_url: Option<String>,
}

impl TomlGithubApp {
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
    git_url: Option<String>,
    secret: String,
    branch: String,
    timeout: Option<u64>,
    bypass: Option<bool>,
    deploy_toml: Option<bool>,
    commit_filter: Option<TomlCommitFilter>,
    commands: TomlCommands,
    auth: Option<TomlProjectAuth>,
}

impl TomlProject {
    pub fn name(&self) -> &str {
        &self.name
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
    pub fn auth(&self) -> Option<&TomlProjectAuth> {
        self.auth.as_ref()
    }
}

#[derive(Deserialize, Clone, Default)]
pub struct TomlProjectOverride {
    pub branch: Option<String>,
    pub timeout: Option<u64>,
    pub bypass: Option<bool>,
    pub commit_filter: Option<TomlCommitFilter>,
    pub commands: Option<TomlCommands>,
    pub auth: Option<TomlProjectAuth>,
}

#[derive(Deserialize, Clone)]
pub struct TomlProjectAuth {
    mode: TomlAuthMode,
    owner: Option<String>,
    repo: Option<String>,
}

impl TomlProjectAuth {
    pub fn mode(&self) -> TomlAuthMode {
        self.mode.clone()
    }
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    pub fn repo(&self) -> Option<&str> {
        self.repo.as_deref()
    }
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TomlAuthMode {
    Ssh,
    GithubApp,
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
    no_pull: Option<bool>,
    init: Option<String>,
    update: Option<String>,
    start: Option<String>,
    restart: Option<String>,
}

impl TomlCommands {
    fn field(&self, name: &str) -> Option<&str> {
        match name {
            "stop" => self.stop.as_deref(),
            "pull" => self.pull.as_deref(),
            "init" => self.init.as_deref(),
            "update" => self.update.as_deref(),
            "start" => self.start.as_deref(),
            "restart" => self.restart.as_deref(),
            _ => None,
        }
    }

    /// Expands every `@field` token found anywhere in `raw` with that field's
    /// own (recursively expanded) command text, e.g. `"@init && restart"`.
    /// `active` tracks the fields currently being expanded on this call stack;
    /// a field that reappears there is a cycle, so its token is left
    /// unexpanded (literal `@name`) instead of recursing forever.
    fn expand(&self, raw: &str, active: &mut Vec<String>) -> String {
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(at) = rest.find('@') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let name_len = after
                .find(|c: char| !(c.is_ascii_alphabetic() || c == '_'))
                .unwrap_or(after.len());
            let name = &after[..name_len];
            let known = self.field(name).is_some() || name.is_empty();
            if name.is_empty() || active.contains(&name.to_string()) || !known {
                out.push('@');
                out.push_str(name);
            } else if let Some(value) = self.field(name) {
                active.push(name.to_string());
                out.push_str(&self.expand(value, active));
                active.pop();
            }
            rest = &after[name_len..];
        }
        out.push_str(rest);
        out
    }

    fn resolve(&self, raw: Option<&str>) -> Option<String> {
        let raw = raw?;
        let mut active = Vec::new();
        Some(self.expand(raw, &mut active))
    }
    pub fn stop(&self) -> Option<String> {
        self.resolve(self.stop.as_deref())
    }
    pub fn pull(&self) -> Option<String> {
        if self.no_pull.unwrap_or(false) {
            return None;
        }
        Some(self.resolve(self.pull.as_deref()).unwrap_or_else(|| {
            "git fetch origin && git reset --hard origin/$(git rev-parse --abbrev-ref HEAD)"
                .to_string()
        }))
    }
    pub fn init(&self) -> Option<String> {
        self.resolve(self.init.as_deref())
    }
    pub fn update(&self) -> Option<String> {
        self.resolve(self.update.as_deref())
    }
    pub fn start(&self) -> Option<String> {
        self.resolve(self.start.as_deref())
    }
    pub fn restart(&self) -> Option<String> {
        self.resolve(self.restart.as_deref())
    }
    pub fn merge_from(&self, other: &TomlCommands) -> TomlCommands {
        TomlCommands {
            stop: other.stop.clone().or_else(|| self.stop.clone()),
            pull: other.pull.clone().or_else(|| self.pull.clone()),
            no_pull: other.no_pull.or(self.no_pull),
            init: other.init.clone().or_else(|| self.init.clone()),
            update: other.update.clone().or_else(|| self.update.clone()),
            start: other.start.clone().or_else(|| self.start.clone()),
            restart: other.restart.clone().or_else(|| self.restart.clone()),
        }
    }
}

#[cfg(test)]
mod command_reference_tests {
    use super::TomlCommands;

    fn commands(
        stop: Option<&str>,
        init: Option<&str>,
        update: Option<&str>,
        start: Option<&str>,
    ) -> TomlCommands {
        TomlCommands {
            stop: stop.map(String::from),
            pull: None,
            no_pull: Some(true),
            init: init.map(String::from),
            update: update.map(String::from),
            start: start.map(String::from),
            restart: None,
        }
    }

    #[test]
    fn resolves_direct_reference() {
        let c = commands(None, Some("cargo build --release"), Some("@init"), None);
        assert_eq!(c.update().as_deref(), Some("cargo build --release"));
    }

    #[test]
    fn resolves_chained_reference() {
        let c = commands(Some("@init"), Some("@update"), Some("cargo build"), None);
        assert_eq!(c.stop().as_deref(), Some("cargo build"));
    }

    #[test]
    fn self_cycle_leaves_token_unexpanded() {
        let c = commands(None, Some("@init"), None, None);
        assert_eq!(c.init().as_deref(), Some("@init"));
    }

    #[test]
    fn two_cycle_leaves_token_unexpanded_without_looping() {
        let c = commands(None, Some("@update"), Some("@init"), None);
        assert_eq!(c.init().as_deref(), Some("@update"));
        assert_eq!(c.update().as_deref(), Some("@init"));
    }

    #[test]
    fn reference_to_unset_field_leaves_token_unexpanded() {
        let c = commands(None, None, Some("@init"), None);
        assert_eq!(c.update().as_deref(), Some("@init"));
    }

    #[test]
    fn expands_multiple_references_inline() {
        let c = commands(
            None,
            Some("cargo build --release"),
            Some("@init && systemctl restart my-api"),
            Some("systemctl start my-api"),
        );
        assert_eq!(
            c.update().as_deref(),
            Some("cargo build --release && systemctl restart my-api")
        );
    }

    #[test]
    fn expands_two_distinct_references_in_one_field() {
        let combined = commands(
            Some("systemctl stop my-api"),
            Some("cargo build --release"),
            Some("@start && @init"),
            Some("systemctl start my-api"),
        );
        assert_eq!(
            combined.update().as_deref(),
            Some("systemctl start my-api && cargo build --release")
        );
    }

    #[test]
    fn unknown_token_left_as_is() {
        let c = commands(None, None, Some("deploy@example.com notify"), None);
        assert_eq!(c.update().as_deref(), Some("deploy@example.com notify"));
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
