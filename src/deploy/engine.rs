use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::AppState;
use crate::configure::{Project, ProjectAuth};
use crate::notify::telegram::NotifyEvent;
use crate::webhook::payload::PushEvent;

use crate::logging::project_log::ProjectLog;

use super::filter::{branch_matches, commit_filter_passes};
use super::runner;
use super::verify::{all_commits_signed, authors_allowed};

pub struct DeployEngine {
    pub project: Arc<Project>,
    pub state: Arc<AppState>,
}

#[derive(Debug, Clone)]
pub enum DeployStep {
    Stop,
    Pull,
    Init,
    Update,
    Start,
    Restart,
    Verify,
}

impl std::fmt::Display for DeployStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop => write!(f, "stop"),
            Self::Pull => write!(f, "pull"),
            Self::Init => write!(f, "init"),
            Self::Update => write!(f, "update"),
            Self::Start => write!(f, "start"),
            Self::Restart => write!(f, "restart"),
            Self::Verify => write!(f, "verify"),
        }
    }
}

pub enum DeployOutcome {
    Success,
    Aborted {
        step: DeployStep,
        reason: String,
    },
    Bypassed,
    #[allow(dead_code)]
    Skipped {
        reason: String,
    },
}

impl DeployEngine {
    pub async fn run(&self, event: PushEvent) {
        let p = &self.project;
        let name = p.name();

        if !branch_matches(&event, p.branch()) {
            info!(
                project = name,
                branch = event.branch_name(),
                expected = p.branch(),
                "branch mismatch, skipping"
            );
            return;
        }

        if let Some(filter) = p.commit_filter() {
            if !commit_filter_passes(event.commits(), filter) {
                info!(project = name, "commit filter excluded this push, skipping");
                return;
            }
        }

        if let Some(verify) = p.commit_verify() {
            if !authors_allowed(event.commits(), verify) {
                warn!(
                    project = name,
                    "commit author not in allowlist, aborting deploy"
                );
                if let Some(notifier) = &self.state.notifier {
                    notifier
                        .send(NotifyEvent::Failed {
                            project: name.to_owned(),
                            step: DeployStep::Verify.to_string(),
                            reason: "commit author not in allowlist".to_owned(),
                        })
                        .await;
                }
                return;
            }

            if verify.require_signed() {
                match all_commits_signed(&event, self.state.github_app.as_deref()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!(project = name, "unsigned commit detected, aborting deploy");
                        if let Some(notifier) = &self.state.notifier {
                            notifier
                                .send(NotifyEvent::Failed {
                                    project: name.to_owned(),
                                    step: DeployStep::Verify.to_string(),
                                    reason: "unsigned commit detected".to_owned(),
                                })
                                .await;
                        }
                        return;
                    }
                    Err(e) => {
                        error!(project = name, error = %e, "failed to verify commit signatures, aborting deploy");
                        if let Some(notifier) = &self.state.notifier {
                            notifier
                                .send(NotifyEvent::Failed {
                                    project: name.to_owned(),
                                    step: DeployStep::Verify.to_string(),
                                    reason: format!("failed to verify commit signatures: {e}"),
                                })
                                .await;
                        }
                        return;
                    }
                }
            }
        }

        info!(project = name, "deploy triggered");

        let start = std::time::Instant::now();
        let commit_summary = (!event.commits().is_empty()).then(|| event.render_commit_summary());
        let mut started_msgs = Vec::new();
        if let Some(notifier) = &self.state.notifier {
            started_msgs = notifier
                .send(NotifyEvent::Started {
                    project: name.to_owned(),
                    commit_summary: commit_summary.clone(),
                })
                .await;
        }

        // Publishes the step `execute` is currently on so the progress ticker can report it.
        let (step_tx, step_rx) = watch::channel(DeployStep::Stop.to_string());
        let progress = (!started_msgs.is_empty())
            .then(|| {
                self.state.notifier.as_ref().map(|notifier| {
                    spawn_progress_ticker(
                        notifier.clone(),
                        started_msgs.clone(),
                        name.to_owned(),
                        commit_summary.clone(),
                        start,
                        step_rx,
                    )
                })
            })
            .flatten();

        let outcome = self.execute(&step_tx).await;

        // Stop the ticker before the terminal edit, so it cannot overwrite the final message.
        if let Some(handle) = progress {
            handle.abort();
            let _ = handle.await;
        }

        match &outcome {
            DeployOutcome::Success => {
                info!(project = name, "deploy succeeded");
                if let Some(notifier) = &self.state.notifier {
                    let elapsed = start.elapsed();
                    if started_msgs.is_empty() {
                        notifier
                            .send(NotifyEvent::Success {
                                project: name.to_owned(),
                                elapsed,
                                commit_summary,
                            })
                            .await;
                    } else {
                        let text = NotifyEvent::Success {
                            project: name.to_owned(),
                            elapsed,
                            commit_summary,
                        }
                        .message();
                        for (chat_id, message_id) in started_msgs.drain(..) {
                            notifier.edit(chat_id, message_id, text.clone()).await;
                        }
                    }
                }
            }
            DeployOutcome::Aborted { step, reason } => {
                error!(project = name, step = %step, reason, "deploy aborted");
                if let Some(notifier) = &self.state.notifier {
                    notifier
                        .send(NotifyEvent::Failed {
                            project: name.to_owned(),
                            step: step.to_string(),
                            reason: reason.clone(),
                        })
                        .await;
                }
            }
            DeployOutcome::Bypassed => {
                info!(project = name, "bypass enabled, deploy skipped");
            }
            DeployOutcome::Skipped { reason } => {
                warn!(project = name, reason, "deploy skipped");
            }
        }
    }

    async fn execute(&self, step_tx: &watch::Sender<String>) -> DeployOutcome {
        let p = &self.project;
        let name = p.name();
        let cwd = p.working_dir();
        let timeout = p.effective_timeout();
        let shell = self.state.shell.as_str();

        if p.bypass() {
            return DeployOutcome::Bypassed;
        }

        let lock_arc = self
            .state
            .locks
            .entry(p.http_path().to_owned())
            .or_default()
            .clone();
        let _guard = if !self.state.no_lock {
            Some(lock_arc.mutex.lock().await)
        } else {
            None
        };

        let use_restart = p.commands().restart().is_some();
        let project_log = ProjectLog::new(&self.state.log_dir, name);

        if !use_restart {
            if let Some(stop_cmd) = p.commands().stop() {
                step_tx.send_replace(DeployStep::Stop.to_string());
                info!(project = name, cmd = stop_cmd, "running stop");
                match runner::run(stop_cmd, cwd, timeout, None, None, shell).await {
                    Err(e) => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Stop,
                            reason: e.to_string(),
                        };
                    }
                    Ok(out) => {
                        project_log.write_command("stop", stop_cmd, &out).await;
                        if !out.success {
                            return DeployOutcome::Aborted {
                                step: DeployStep::Stop,
                                reason: out.failure_summary(name),
                            };
                        }
                        info!(project = name, "stop completed");
                    }
                }
            }
        }

        if let Some(pull_cmd) = p.commands().pull() {
            let gh_token = match p.auth() {
                ProjectAuth::GithubApp { owner, repo } => {
                    let auth = self
                        .state
                        .github_app
                        .as_ref()
                        .expect("github_app auth validated at config load time");
                    match auth.get_token(owner, repo).await {
                        Ok(token) => Some(token),
                        Err(e) => {
                            return DeployOutcome::Aborted {
                                step: DeployStep::Pull,
                                reason: format!("failed to obtain GitHub App token: {e}"),
                            };
                        }
                    }
                }
                ProjectAuth::Ssh => None,
            };
            let extra_env = gh_token.as_deref().map(|t| ("GH_TOKEN", t));
            let auth_header = gh_token.as_deref().map(super::git::basic_auth_header_value);
            let extra_git_config = auth_header.as_deref().map(|v| ("http.extraHeader", v));

            step_tx.send_replace(DeployStep::Pull.to_string());
            info!(project = name, cmd = pull_cmd, "running pull");
            match runner::run(pull_cmd, cwd, timeout, extra_env, extra_git_config, shell).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Pull,
                        reason: e.to_string(),
                    };
                }
                Ok(out) => {
                    project_log.write_command("pull", pull_cmd, &out).await;
                    if !out.success {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Pull,
                            reason: out.failure_summary(name),
                        };
                    }
                    info!(project = name, "pull completed");
                }
            }
        }

        let is_first = !self.state.state.is_initialized(name).await;
        if is_first {
            if let Some(init_cmd) = p.commands().init() {
                step_tx.send_replace(DeployStep::Init.to_string());
                info!(
                    project = name,
                    cmd = init_cmd,
                    "running init (first deploy)"
                );
                match runner::run(init_cmd, cwd, timeout, None, None, shell).await {
                    Err(e) => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Init,
                            reason: e.to_string(),
                        };
                    }
                    Ok(out) => {
                        project_log.write_command("init", init_cmd, &out).await;
                        if !out.success {
                            return DeployOutcome::Aborted {
                                step: DeployStep::Init,
                                reason: out.failure_summary(name),
                            };
                        }
                        info!(project = name, "init completed");
                        if let Err(e) = self.state.state.mark_initialized(name).await {
                            warn!(project = name, error = %e, "failed to persist init state");
                        }
                    }
                }
            } else if let Err(e) = self.state.state.mark_initialized(name).await {
                warn!(project = name, error = %e, "failed to persist init state");
            }
        }

        if let Some(update_cmd) = p.commands().update() {
            step_tx.send_replace(DeployStep::Update.to_string());
            info!(project = name, cmd = update_cmd, "running update");
            match runner::run(update_cmd, cwd, timeout, None, None, shell).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Update,
                        reason: e.to_string(),
                    };
                }
                Ok(out) => {
                    project_log.write_command("update", update_cmd, &out).await;
                    if !out.success {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Update,
                            reason: out.failure_summary(name),
                        };
                    }
                    info!(project = name, "update completed");
                }
            }
        }

        if use_restart {
            let restart_cmd = p.commands().restart().unwrap();
            step_tx.send_replace(DeployStep::Restart.to_string());
            info!(project = name, cmd = restart_cmd, "running restart");
            match runner::run(restart_cmd, cwd, timeout, None, None, shell).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Restart,
                        reason: e.to_string(),
                    };
                }
                Ok(out) => {
                    project_log
                        .write_command("restart", restart_cmd, &out)
                        .await;
                    if !out.success {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Restart,
                            reason: out.failure_summary(name),
                        };
                    }
                    info!(project = name, "restart completed");
                }
            }
        } else if let Some(start_cmd) = p.commands().start() {
            step_tx.send_replace(DeployStep::Start.to_string());
            info!(project = name, cmd = start_cmd, "running start");
            match runner::run(start_cmd, cwd, timeout, None, None, shell).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Start,
                        reason: e.to_string(),
                    };
                }
                Ok(out) => {
                    project_log.write_command("start", start_cmd, &out).await;
                    if !out.success {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Start,
                            reason: out.failure_summary(name),
                        };
                    }
                    info!(project = name, "start completed");
                }
            }
        }

        DeployOutcome::Success
    }
}

const PROGRESS_INTERVAL: Duration = Duration::from_secs(60);

/// Edits the "deploy started" message once a minute with the elapsed time and the
/// step currently running, until aborted by the caller at deploy completion.
fn spawn_progress_ticker(
    notifier: crate::notify::telegram::TelegramNotifier,
    targets: Vec<(teloxide::types::ChatId, teloxide::types::MessageId)>,
    project: String,
    commit_summary: Option<String>,
    start: std::time::Instant,
    step_rx: watch::Receiver<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PROGRESS_INTERVAL);
        // The first tick completes immediately; skip it so the first edit lands at 1 minute.
        interval.tick().await;

        loop {
            interval.tick().await;

            let text = NotifyEvent::Progress {
                project: project.clone(),
                step: step_rx.borrow().clone(),
                elapsed: start.elapsed(),
                commit_summary: commit_summary.clone(),
            }
            .message();

            for (chat_id, message_id) in &targets {
                notifier.edit(*chat_id, *message_id, text.clone()).await;
            }
        }
    })
}
