use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use tracing::{info, warn};

use crate::deploy::engine::DeployEngine;
use crate::AppState;

use super::payload::{PingEvent, PushEvent};
use super::signature;

const MAX_BODY: usize = 256 * 1024;

fn is_all_zeros(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c == '0')
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if body.len() > MAX_BODY {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body too large").into_response();
    }

    let path = headers
        .get("X-Original-Path")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let project = match state.projects.get(path) {
        Some(p) => p.clone(),
        None => return (StatusCode::NOT_FOUND, "unknown project").into_response(),
    };

    let sig_header = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !signature::verify(project.secret(), &body, sig_header) {
        warn!(project = project.name(), "HMAC verification failed");
        return (StatusCode::FORBIDDEN, "signature mismatch").into_response();
    }

    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    match event_type {
        "ping" => {
            let event: PingEvent = match serde_json::from_slice(&body) {
                Ok(e) => e,
                Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
            };
            info!(project = project.name(), zen = event.zen(), "ping received");
            return StatusCode::OK.into_response();
        }
        "push" => {}
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unsupported event: {other}"),
            )
                .into_response();
        }
    }

    let event: PushEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    if is_all_zeros(event.after()) || is_all_zeros(event.before()) {
        info!(
            project = project.name(),
            "skipping branch create/delete event"
        );
        return StatusCode::OK.into_response();
    }

    let delivery_id = headers
        .get("X-GitHub-Delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let lock = state
        .locks
        .entry(project.http_path().to_owned())
        .or_default();
    if !delivery_id.is_empty() {
        if lock.seen.contains(&delivery_id) {
            info!(
                project = project.name(),
                delivery_id, "duplicate delivery, skipping"
            );
            return StatusCode::OK.into_response();
        }
        lock.seen.insert(delivery_id);
    }

    let engine = DeployEngine {
        project: project.clone(),
        state: state.clone(),
    };

    tokio::spawn(async move {
        engine.run(event).await;
    });

    StatusCode::OK.into_response()
}
