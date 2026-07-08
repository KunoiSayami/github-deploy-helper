use std::collections::HashMap;
use std::sync::Arc;

use tracing::info;

use crate::auth::github_app::GithubAppAuth;

use super::{GithubAppConfig, Project, ProjectAuth};

/// Ensures every project's `working_dir` exists: clones it if `git_url` is
/// configured, otherwise just creates the directory so deploy commands have a
/// valid cwd. Used both at startup and after a config reload introduces a new
/// project.
pub async fn clone_missing_projects(
    projects: &HashMap<String, Arc<Project>>,
    github_app: Option<&GithubAppAuth>,
) {
    for project in projects.values() {
        if std::path::Path::new(project.working_dir()).exists() {
            continue;
        }
        let Some(git_url) = project.git_url() else {
            // Not git-managed (e.g. no_pull webhook-only project): just make sure
            // working_dir exists so deploy commands have a valid cwd to run in.
            if let Err(e) = std::fs::create_dir_all(project.working_dir()) {
                tracing::error!(project = project.name(), path = project.working_dir(), error = %e, "failed to create working_dir");
            }
            continue;
        };

        let gh_token = match project.auth() {
            ProjectAuth::GithubApp { owner, repo } => {
                let Some(app) = github_app else {
                    tracing::error!(
                        project = project.name(),
                        "cannot auto-clone: auth.mode = \"github_app\" but no [github_app] config is present"
                    );
                    continue;
                };
                match app.get_token(owner, repo).await {
                    Ok(token) => Some(token),
                    Err(e) => {
                        tracing::error!(project = project.name(), error = %e, "auto-clone: failed to obtain GitHub App token");
                        continue;
                    }
                }
            }
            ProjectAuth::Ssh => None,
        };

        info!(
            project = project.name(),
            path = project.working_dir(),
            "working_dir missing, cloning"
        );
        if let Err(e) = crate::deploy::git::clone(
            git_url,
            project.working_dir(),
            project.branch(),
            gh_token.as_deref(),
        )
        .await
        {
            tracing::error!(project = project.name(), error = %e, "auto-clone failed");
        }
    }
}

/// Ensures every GitHub-App-authed project has a webhook registered pointing
/// at `public_base_url`. Used both at startup and after a config reload.
pub async fn ensure_webhooks(
    projects: &HashMap<String, Arc<Project>>,
    github_app: Option<&GithubAppAuth>,
    github_app_config: Option<&GithubAppConfig>,
) {
    let (Some(app), Some(base_url)) = (
        github_app,
        github_app_config.and_then(|g| g.public_base_url()),
    ) else {
        return;
    };

    for project in projects.values() {
        if let ProjectAuth::GithubApp { owner, repo } = project.auth() {
            let webhook_url = format!("{}{}", base_url.trim_end_matches('/'), project.http_path());
            if let Err(e) = app
                .ensure_webhook(owner, repo, &webhook_url, project.secret())
                .await
            {
                tracing::warn!(
                    project = project.name(),
                    error = %e,
                    "failed to auto-configure GitHub webhook"
                );
            }
        }
    }
}
