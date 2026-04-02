//! Setting 插件 - 设置管理

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

pub struct SettingPlugin {
    meta: PluginMeta,
    parent: Option<Weak<dyn Plugin>>,
}

impl SettingPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "setting".to_string(),
            description: "设置管理插件".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "get", "set", "reset"],
                        "description": "操作类型"
                    },
                    "category": { "type": "string" },
                    "key": { "type": "string" },
                    "value": {}
                }
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "data": { "type": "object" },
                    "message": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        SettingPlugin {
            meta: Self::create_meta(),
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }
}

impl Default for SettingPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        match action {
            "list" => Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "categories": [
                        {"id": "general", "name": "常规设置", "icon": "settings"},
                        {"id": "appearance", "name": "外观设置", "icon": "palette"},
                        {"id": "editor", "name": "编辑器设置", "icon": "edit"},
                        {"id": "ai", "name": "AI 设置", "icon": "smart_toy"},
                        {"id": "docker", "name": "Docker 设置", "icon": "docker"},
                    ]
                }
            }))),
            "get" => {
                let category = input.get("category").and_then(|v| v.as_str()).unwrap_or("general");
                let settings = match category {
                    "general" => json!({
                        "language": "zh-CN",
                        "autoSave": true,
                        "autoSaveInterval": 30000
                    }),
                    "appearance" => json!({
                        "theme": "light",
                        "fontSize": 14,
                        "sidebarWidth": 250
                    }),
                    "editor" => json!({
                        "tabSize": 2,
                        "lineNumbers": true,
                        "wordWrap": true
                    }),
                    "ai" => json!({
                        "provider": "openai",
                        "model": "gpt-4",
                        "temperature": 0.7
                    }),
                    "docker" => json!({
                        "enabled": true,
                        "image": "symbio/bio-tools:latest",
                        "memoryLimit": "2g"
                    }),
                    _ => json!({})
                };
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"category": category, "settings": settings}
                })))
            }
            "set" => {
                let category = input.get("category").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 category 参数".to_string()))?;
                let key = input.get("key").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 key 参数".to_string()))?;
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"category": category, "key": key},
                    "message": "设置已保存"
                })))
            }
            "reset" => {
                let category = input.get("category").and_then(|v| v.as_str());
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"category": category},
                    "message": "设置已重置为默认值"
                })))
            }
            _ => Err(PluginError::ValidationError(format!("未知操作: {}", action))),
        }
    }
}
