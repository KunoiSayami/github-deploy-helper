use std::time::Duration;

use teloxide::prelude::*;
use teloxide::sugar::request::RequestLinkPreviewExt;
use teloxide::types::{ChatId, MessageId, ParseMode};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, warn};

pub enum NotifyEvent {
    Started {
        project: String,
        commit_summary: Option<String>,
    },
    Progress {
        project: String,
        step: String,
        elapsed: Duration,
        commit_summary: Option<String>,
    },
    Success {
        project: String,
        elapsed: Duration,
        commit_summary: Option<String>,
    },
    Failed {
        project: String,
        step: String,
        reason: String,
    },
}

impl NotifyEvent {
    pub fn message(&self) -> String {
        match self {
            Self::Started {
                project,
                commit_summary,
            } => {
                let project = html_escape(project);
                match commit_summary {
                    Some(summary) => format!("🚀 <b>{project}</b>: deploy started\n\n{summary}"),
                    None => format!("🚀 <b>{project}</b>: deploy started"),
                }
            }
            Self::Progress {
                project,
                step,
                elapsed,
                commit_summary,
            } => {
                let title = format!(
                    "🚀 <b>{}</b>: deploy started\n\nrunning <code>{}</code>… ({} elapsed)",
                    html_escape(project),
                    html_escape(step),
                    format_elapsed(*elapsed)
                );
                match commit_summary {
                    Some(summary) => format!("{title}\n\n{summary}"),
                    None => title,
                }
            }
            Self::Success {
                project,
                elapsed,
                commit_summary,
            } => {
                let title = format!(
                    "✅ <b>{}</b>: deploy succeeded in {}",
                    html_escape(project),
                    format_elapsed(*elapsed)
                );
                match commit_summary {
                    Some(summary) => format!("{title}\n\n{summary}"),
                    None => title,
                }
            }
            Self::Failed {
                project,
                step,
                reason,
            } => {
                let project = html_escape(project);
                let step = html_escape(step);
                let escaped = html_escape(reason);
                format!(
                    "❌ <b>{project}</b>: deploy failed at <code>{step}</code>\n<pre>{escaped}</pre>"
                )
            }
        }
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs == 0 {
        format!("{}ms", elapsed.as_millis())
    } else if secs < 60 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

enum Cmd {
    Send(String, oneshot::Sender<Vec<(ChatId, MessageId)>>),
    Edit(ChatId, MessageId, String),
    Stop,
}

#[derive(Clone)]
pub struct TelegramNotifier {
    tx: mpsc::Sender<Cmd>,
}

impl TelegramNotifier {
    pub async fn send(&self, event: NotifyEvent) -> Vec<(ChatId, MessageId)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .tx
            .send(Cmd::Send(event.message(), reply_tx))
            .await
            .is_err()
        {
            return Vec::new();
        }
        reply_rx.await.unwrap_or_default()
    }

    pub async fn edit(&self, chat_id: ChatId, message_id: MessageId, text: String) {
        let _ = self.tx.send(Cmd::Edit(chat_id, message_id, text)).await;
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Stop).await;
    }
}

pub fn start(bot_token: String, api_server: Option<String>, send_to: Vec<i64>) -> TelegramNotifier {
    let (tx, mut rx) = mpsc::channel::<Cmd>(256);

    tokio::spawn(async move {
        if bot_token.is_empty() {
            warn!("Telegram bot token is empty, notifications disabled");
            while let Some(cmd) = rx.recv().await {
                if let Cmd::Stop = cmd {
                    break;
                }
            }
            return;
        }

        let bot = Bot::new(&bot_token);
        let bot = match api_server {
            Some(ref api) => match api.parse() {
                Ok(url) => bot.set_api_url(url),
                Err(e) => {
                    error!("Invalid Telegram api_server URL: {e}");
                    bot
                }
            },
            None => bot,
        };
        let bot = bot.parse_mode(ParseMode::Html);

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Cmd::Stop => break,
                Cmd::Send(text, reply_tx) => {
                    let mut sent = Vec::with_capacity(send_to.len());
                    for &chat_id in &send_to {
                        let chat_id = ChatId(chat_id);
                        let request = bot.send_message(chat_id, &text).disable_link_preview(true);

                        match request.await {
                            Ok(message) => sent.push((chat_id, message.id)),
                            Err(e) => error!("Telegram send_message error: {e}"),
                        }
                    }
                    let _ = reply_tx.send(sent);
                }
                Cmd::Edit(chat_id, message_id, text) => {
                    let request = bot
                        .edit_message_text(chat_id, message_id, &text)
                        .disable_link_preview(true);

                    if let Err(e) = request.await {
                        error!(
                            "Telegram edit_message_text error: {e}, falling back to new message"
                        );
                        let request = bot.send_message(chat_id, &text).disable_link_preview(true);
                        if let Err(e) = request.await {
                            error!("Telegram send_message error: {e}");
                        }
                    }
                }
            }
        }
    });

    TelegramNotifier { tx }
}
