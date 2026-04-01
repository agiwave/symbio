//! Memory 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::{MemoryPlugin, MemoryConfig};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct MemoryFactory;

impl MemoryFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// 规范化配置，处理缺失字段
fn normalize_config(value: &Value) -> MemoryConfig {
    if let Ok(config) = serde_json::from_value::<MemoryConfig>(value.clone()) {
        return config;
    }
    let mut config = MemoryConfig::default();
    if let Some(obj) = value.as_object() {
        if let Some(v) = obj.get("storage_dir").and_then(|v| v.as_str()) {
            config.storage_dir = v.to_string();
        }
        if let Some(v) = obj.get("max_entries").and_then(|v| v.as_u64()) {
            config.max_entries = v as usize;
        }
        if let Some(v) = obj.get("categories").and_then(|v| v.as_array()) {
            config.categories = v.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect();
        }
    }
    config
}

impl PluginFactory for MemoryFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "memory".to_string(),
            description: "持久化记忆存储".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["store", "recall", "forget", "list", "search"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Arc<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin> {
        let memory_config = config
            .map(normalize_config)
            .unwrap_or_default();
        
        let parent_weak = parent.as_ref().map(|p| Arc::downgrade(p));
        Arc::new(MemoryPlugin::new(parent_weak, memory_config))
    }
}
