//! Telegram 类型定义

use serde::{Deserialize, Serialize};

/// Telegram Bot 配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
    #[serde(default = "default_true")]
    pub poll_enabled: bool,
    /// 启用流式响应
    #[serde(default = "default_true")]
    pub streaming_enabled: bool,
    /// 显示思考内容
    #[serde(default = "default_true")]
    pub show_thinking: bool,
    #[serde(default)]
    pub use_webhook: bool,
    pub webhook_url: Option<String>,
}

fn default_true() -> bool {
    true
}

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

/// Telegram 更新
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramIncomingMessage>,
}

/// Telegram 收到的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramIncomingMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub text: Option<String>,
    pub date: i64,
}

/// Telegram 用户
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

/// Telegram 聊天
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
    pub username: Option<String>,
}

/// Telegram API 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error_code: Option<i32>,
    pub description: Option<String>,
}
