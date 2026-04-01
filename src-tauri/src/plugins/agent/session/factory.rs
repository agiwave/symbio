//! Session 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::{SessionPlugin, SessionConfig};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct SessionFactory;

impl SessionFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SessionFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// 规范化配置，处理缺失字段
fn normalize_config(value: &Value) -> SessionConfig {
    if let Ok(config) = serde_json::from_value::<SessionConfig>(value.clone()) {
        return config;
    }
    let mut config = SessionConfig::default();
    if let Some(obj) = value.as_object() {
        if let Some(v) = obj.get("storage_dir").and_then(|v| v.as_str()) {
            config.storage_dir = v.to_string();
        }
        if let Some(v) = obj.get("max_messages").and_then(|v| v.as_u64()) {
            config.max_messages = v as usize;
        }
        if let Some(v) = obj.get("auto_compress").and_then(|v| v.as_bool()) {
            config.auto_compress = v;
        }
        if let Some(v) = obj.get("compress_threshold").and_then(|v| v.as_u64()) {
            config.compress_threshold = v as usize;
        }
    }
    config
}

impl PluginFactory for SessionFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "session".to_string(),
            description: "会话历史和上下文管理".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "append", "clear", "list", "get_context"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Arc<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin> {
        let session_config = config
            .map(normalize_config)
            .unwrap_or_default();
        
        let parent_weak = parent.as_ref().map(|p| Arc::downgrade(p));
        Arc::new(SessionPlugin::new(parent_weak, session_config))
    }
}