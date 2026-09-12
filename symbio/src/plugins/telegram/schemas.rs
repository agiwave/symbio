//! Telegram 插件的配置 schema（只被本插件消费，故定义在插件内部）

use serde::{Deserialize, Serialize};

/// Telegram configuration - Single Source of Truth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: Option<String>,
    #[serde(default = "default_true")]
    pub streaming_enabled: bool,
    #[serde(default = "default_true")]
    pub poll_enabled: bool,
    #[serde(default)]
    pub allowed_users: Vec<i64>,
}

fn default_true() -> bool {
    true
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: "".to_string(),
            chat_id: None,
            streaming_enabled: true,
            poll_enabled: true,
            allowed_users: Vec::new(),
        }
    }
}

pub mod telegram_send {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Request {
        pub text: String,
        pub chat_id: Option<String>,
        pub parse_mode: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub sent: i32,
        pub message: String,
    }
}

pub mod telegram_status {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub configured: bool,
        pub has_chat_id: bool,
        pub streaming_enabled: bool,
        pub poll_enabled: bool,
        pub listener_running: bool,
        pub update_offset: i64,
    }
}
