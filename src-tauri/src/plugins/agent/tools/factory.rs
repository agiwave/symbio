//! Tools 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::{ToolsPlugin, ToolsConfig};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ToolsFactory;

impl ToolsFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ToolsFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// 规范化配置，处理缺失字段
fn normalize_config(value: &Value) -> ToolsConfig {
    // 尝试直接解析
    if let Ok(config) = serde_json::from_value::<ToolsConfig>(value.clone()) {
        return config;
    }
    // 解析失败则使用默认值合并
    let mut config = ToolsConfig::default();
    if let Some(obj) = value.as_object() {
        if let Some(v) = obj.get("shell_enabled").and_then(|v| v.as_bool()) {
            config.shell_enabled = v;
        }
        if let Some(v) = obj.get("file_enabled").and_then(|v| v.as_bool()) {
            config.file_enabled = v;
        }
        if let Some(v) = obj.get("web_enabled").and_then(|v| v.as_bool()) {
            config.web_enabled = v;
        }
        if let Some(v) = obj.get("allowed_paths").and_then(|v| v.as_array()) {
            config.allowed_paths = v.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(v) = obj.get("blocked_commands").and_then(|v| v.as_array()) {
            config.blocked_commands = v.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(v) = obj.get("shell_timeout").and_then(|v| v.as_u64()) {
            config.shell_timeout = v;
        }
        if let Some(v) = obj.get("web_timeout").and_then(|v| v.as_u64()) {
            config.web_timeout = v;
        }
    }
    config
}

impl PluginFactory for ToolsFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "tools".to_string(),
            description: "文件操作和 Shell 命令工具".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "tool": {"type": "string"},
                    "params": {"type": "object"}
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Arc<dyn Plugin>>, config: Option<&Value>) -> Arc<dyn Plugin> {
        let tools_config = config
            .map(normalize_config)
            .unwrap_or_default();
        
        let parent_weak = parent.as_ref().map(|p| Arc::downgrade(p));
        Arc::new(ToolsPlugin::new(parent_weak, tools_config))
    }
}
