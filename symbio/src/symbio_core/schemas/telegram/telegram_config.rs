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
