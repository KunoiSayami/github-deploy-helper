use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};
use tokio::sync::mpsc;
use tracing::{error, warn};

pub enum NotifyEvent {
    Started {
        project: String,
    },
    Success {
        project: String,
    },
    Failed {
        project: String,
        step: String,
        reason: String,
    },
}

impl NotifyEvent {
    fn message(&self) -> String {
        match self {
            Self::Started { project } => format!("🚀 <b>{project}</b>: deploy started"),
            Self::Success { project } => format!("✅ <b>{project}</b>: deploy succeeded"),
            Self::Failed {
                project,
                step,
                reason,
            } => {
                let escaped = html_escape(reason);
                format!(
                    "❌ <b>{project}</b>: deploy failed at <code>{step}</code>\n<pre>{escaped}</pre>"
                )
            }
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

enum Cmd {
    Send(String),
    Stop,
}

#[derive(Clone)]
pub struct TelegramNotifier {
    tx: mpsc::Sender<Cmd>,
}

impl TelegramNotifier {
    pub async fn send(&self, event: NotifyEvent) {
        let _ = self.tx.send(Cmd::Send(event.message())).await;
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
                Cmd::Send(text) => {
                    for &chat_id in &send_to {
                        if let Err(e) = bot.send_message(ChatId(chat_id), &text).await {
                            error!("Telegram send_message error: {e}");
                        }
                    }
                }
            }
        }
    });

    TelegramNotifier { tx }
}
