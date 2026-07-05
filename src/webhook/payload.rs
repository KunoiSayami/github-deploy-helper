use serde::Deserialize;

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Deserialize, Debug)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    git_ref: String,
    after: String,
    before: String,
    #[serde(default)]
    commits: Vec<CommitInfo>,
    compare: String,
    repository: RepositoryInfo,
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

    /// Render an HTML-formatted summary of the commits in this push, suitable
    /// for inclusion in a Telegram notification message.
    pub fn render_commit_summary(&self) -> String {
        let git_ref = format!(
            "{}:{}",
            escape_html(self.repository.full_name()),
            escape_html(self.branch_name())
        );
        if let [commit] = self.commits.as_slice() {
            let url = escape_html(commit.url());
            let commit = commit.display();
            format!("🔨 <a href=\"{url}\">1 new commit</a> <b>to {git_ref}</b>:\n\n{commit}")
        } else {
            let url = escape_html(&self.compare);
            let count = self.commits.len();
            let commits = self
                .commits
                .iter()
                .map(CommitInfo::display)
                .collect::<Vec<String>>()
                .join("\n");
            format!(
                "🔨 <a href=\"{url}\">{count} new commits</a> <b>to {git_ref}</b>:\n\n{commits}"
            )
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct RepositoryInfo {
    full_name: String,
}

impl RepositoryInfo {
    pub fn full_name(&self) -> &str {
        &self.full_name
    }
}

#[derive(Deserialize, Debug, Default)]
pub struct CommitInfo {
    #[allow(dead_code)]
    id: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    url: String,
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
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn all_files(&self) -> impl Iterator<Item = &str> {
        self.added
            .iter()
            .chain(self.removed.iter())
            .chain(self.modified.iter())
            .map(String::as_str)
    }

    /// Render this commit as `<a href="...">shortid</a>: first line of message`.
    fn display(&self) -> String {
        let title = self
            .message
            .split_once('\n')
            .map_or(self.message.as_str(), |(t, _)| t);
        let short_id = if self.id.len() >= 8 {
            &self.id[..8]
        } else {
            &self.id
        };
        let url = escape_html(&self.url);
        let title = escape_html(title);
        format!("<a href=\"{url}\">{short_id}</a>: {title}")
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
