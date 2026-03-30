//! Telegram 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::TelegramPlugin;
use super::types::TelegramConfig;
use serde_json::json;
use std::sync::Arc;

pub struct TelegramFactory;

impl TelegramFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TelegramFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for TelegramFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "telegram".to_string(),
            description: "Telegram Bot API 集成".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["send", "get_updates", "configure", "get_config", "set_chat_id"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, config: Option<&serde_json::Value>) -> Arc<dyn Plugin> {
        let telegram_config: TelegramConfig = config
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        
        Arc::new(TelegramPlugin::new(telegram_config))
    }
}
