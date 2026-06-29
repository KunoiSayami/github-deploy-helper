use std::sync::atomic::Ordering;
use std::sync::Arc;

use tracing::{error, info, warn};

use crate::configure::Project;
use crate::notify::telegram::NotifyEvent;
use crate::webhook::payload::PushEvent;
use crate::AppState;

use super::filter::{branch_matches, commit_filter_passes};
use super::runner;

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

        info!(project = name, "deploy triggered");

        if let Some(notifier) = &self.state.notifier {
            notifier
                .send(NotifyEvent::Started {
                    project: name.to_owned(),
                })
                .await;
        }

        let outcome = self.execute().await;

        match &outcome {
            DeployOutcome::Success => {
                info!(project = name, "deploy succeeded");
                if let Some(notifier) = &self.state.notifier {
                    notifier
                        .send(NotifyEvent::Success {
                            project: name.to_owned(),
                        })
                        .await;
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

    async fn execute(&self) -> DeployOutcome {
        let p = &self.project;
        let name = p.name();
        let cwd = p.working_dir();
        let timeout = p.effective_timeout();

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

        if !use_restart {
            if let Some(stop_cmd) = p.commands().stop() {
                info!(project = name, cmd = stop_cmd, "running stop");
                match runner::run(stop_cmd, cwd, timeout).await {
                    Err(e) => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Stop,
                            reason: e.to_string(),
                        }
                    }
                    Ok(out) if !out.success => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Stop,
                            reason: format!("exit failure\nstderr: {}", out.stderr),
                        }
                    }
                    Ok(out) => {
                        info!(project = name, stdout = out.stdout.trim(), "stop completed");
                    }
                }
            }
        }

        let pull_cmd = p.commands().pull();
        info!(project = name, cmd = pull_cmd, "running pull");
        match runner::run(pull_cmd, cwd, timeout).await {
            Err(e) => {
                return DeployOutcome::Aborted {
                    step: DeployStep::Pull,
                    reason: e.to_string(),
                }
            }
            Ok(out) if !out.success => {
                return DeployOutcome::Aborted {
                    step: DeployStep::Pull,
                    reason: format!("exit failure\nstderr: {}", out.stderr),
                }
            }
            Ok(out) => {
                info!(project = name, stdout = out.stdout.trim(), "pull completed");
            }
        }

        let is_first = p.first_deploy.load(Ordering::Acquire);
        if is_first {
            if let Some(init_cmd) = p.commands().init() {
                info!(
                    project = name,
                    cmd = init_cmd,
                    "running init (first deploy)"
                );
                match runner::run(init_cmd, cwd, timeout).await {
                    Err(e) => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Init,
                            reason: e.to_string(),
                        }
                    }
                    Ok(out) if !out.success => {
                        return DeployOutcome::Aborted {
                            step: DeployStep::Init,
                            reason: format!("exit failure\nstderr: {}", out.stderr),
                        }
                    }
                    Ok(out) => {
                        info!(project = name, stdout = out.stdout.trim(), "init completed");
                        p.first_deploy.store(false, Ordering::Release);
                    }
                }
            } else {
                p.first_deploy.store(false, Ordering::Release);
            }
        }

        if let Some(update_cmd) = p.commands().update() {
            info!(project = name, cmd = update_cmd, "running update");
            match runner::run(update_cmd, cwd, timeout).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Update,
                        reason: e.to_string(),
                    }
                }
                Ok(out) if !out.success => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Update,
                        reason: format!("exit failure\nstderr: {}", out.stderr),
                    }
                }
                Ok(out) => {
                    info!(
                        project = name,
                        stdout = out.stdout.trim(),
                        "update completed"
                    );
                }
            }
        }

        if use_restart {
            let restart_cmd = p.commands().restart().unwrap();
            info!(project = name, cmd = restart_cmd, "running restart");
            match runner::run(restart_cmd, cwd, timeout).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Restart,
                        reason: e.to_string(),
                    }
                }
                Ok(out) if !out.success => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Restart,
                        reason: format!("exit failure\nstderr: {}", out.stderr),
                    }
                }
                Ok(out) => {
                    info!(
                        project = name,
                        stdout = out.stdout.trim(),
                        "restart completed"
                    );
                }
            }
        } else if let Some(start_cmd) = p.commands().start() {
            info!(project = name, cmd = start_cmd, "running start");
            match runner::run(start_cmd, cwd, timeout).await {
                Err(e) => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Start,
                        reason: e.to_string(),
                    }
                }
                Ok(out) if !out.success => {
                    return DeployOutcome::Aborted {
                        step: DeployStep::Start,
                        reason: format!("exit failure\nstderr: {}", out.stderr),
                    }
                }
                Ok(out) => {
                    info!(
                        project = name,
                        stdout = out.stdout.trim(),
                        "start completed"
                    );
                }
            }
        }

        DeployOutcome::Success
    }
}
