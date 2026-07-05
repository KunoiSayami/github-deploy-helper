use std::collections::HashMap;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    projects: HashMap<String, ProjectState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectState {
    #[serde(default)]
    initialized: bool,
}

pub struct SharedState {
    inner: RwLock<State>,
    state_file: String,
}

impl SharedState {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let state = if std::path::Path::new(path).exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read state file {path}"))?;
            serde_json::from_str(&content).context("failed to parse state.json")?
        } else {
            State::default()
        };
        Ok(Self {
            inner: RwLock::new(state),
            state_file: path.to_owned(),
        })
    }

    /// Whether `project` has completed its init step in a previous run.
    pub async fn is_initialized(&self, project: &str) -> bool {
        self.inner
            .read()
            .await
            .projects
            .get(project)
            .is_some_and(|p| p.initialized)
    }

    pub async fn mark_initialized(&self, project: &str) -> anyhow::Result<()> {
        self.set_initialized(project, true).await
    }

    /// Forces `project` to re-run its init step on the next deploy.
    pub async fn clear_initialized(&self, project: &str) -> anyhow::Result<()> {
        self.set_initialized(project, false).await
    }

    async fn set_initialized(&self, project: &str, initialized: bool) -> anyhow::Result<()> {
        let mut guard = self.inner.write().await;
        guard
            .projects
            .entry(project.to_owned())
            .or_default()
            .initialized = initialized;
        let json = serde_json::to_string_pretty(&*guard).context("failed to serialize state")?;
        drop(guard);
        std::fs::write(&self.state_file, json)
            .with_context(|| format!("failed to write state file {}", self.state_file))?;
        Ok(())
    }
}
