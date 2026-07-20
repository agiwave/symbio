use serde::{Deserialize, Serialize};

pub use crate::symbio_core::schemas::telegram::telegram_config::TelegramConfig;

/// Telegram 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramMessage {
    pub chat_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
}
