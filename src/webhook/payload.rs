use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    after: String,
    before: String,
    #[serde(default)]
    commits: Vec<CommitInfo>,
}

impl PushEvent {
    #[allow(dead_code)]
    pub fn git_ref(&self) -> &str {
        &self.git_ref
    }
    pub fn after(&self) -> &str {
        &self.after
    }
    pub fn before(&self) -> &str {
        &self.before
    }
    pub fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }
    pub fn branch_name(&self) -> &str {
        self.git_ref
            .rsplit_once('/')
            .map(|(_, b)| b)
            .unwrap_or(&self.git_ref)
    }
}

#[derive(Deserialize, Debug, Default)]
pub struct CommitInfo {
    #[allow(dead_code)]
    id: String,
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    removed: Vec<String>,
    #[serde(default)]
    modified: Vec<String>,
}

impl CommitInfo {
    #[allow(dead_code)]
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn all_files(&self) -> impl Iterator<Item = &str> {
        self.added
            .iter()
            .chain(self.removed.iter())
            .chain(self.modified.iter())
            .map(String::as_str)
    }
}

#[derive(Deserialize, Debug)]
pub struct PingEvent {
    zen: String,
}

impl PingEvent {
    pub fn zen(&self) -> &str {
        &self.zen
    }
}
