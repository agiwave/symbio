//! Aggregation tools - 工具搜索和列表功能
//!
//! 提供 list_all_tools 和 search_tools 功能，用于 Session 集成

use crate::core::traits::Plugin;
use crate::core::types::{PluginError, InvokeStream};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

/// 对 LLM 隐藏的工具名称（内部/管理工具）
const HIDDEN_TOOLS: &[&str] = &[
    "add",
    "remove",
    "status",
    "_get_config",
    "_set_config",
    "_get_meta",
];

/// 检查工具名称是否应该对 LLM 隐藏
fn is_hidden_tool(name: &str) -> bool {
    HIDDEN_TOOLS.contains(&name) || name.starts_with('_')
}

/// Aggregation Tools - 工具搜索功能
pub struct AggregationTools {
    /// 父插件引用（Agent，用于获取所有子插件）
    parent: Option<Weak<dyn Plugin>>,
}

impl AggregationTools {
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self { parent }
    }

    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// 列出所有可用工具
    /// 返回最小信息: name + description
    /// 加上 search_tools 函数用于获取详细 schema
    pub async fn list_all_tools(&self) -> Result<Value, PluginError> {
        let parent = self.get_parent()
            .ok_or_else(|| PluginError::NotFound("没有父插件引用".to_string()))?;

        let mut all_tools: Vec<Value> = Vec::new();

        // 获取 Agent 的所有子插件
        let result = parent.invoke("", json!({}))?;
        let plugins_info = match result {
            crate::core::types::InvokeStream::Single(chunk) => chunk.data,
            _ => return Ok(json!({"tools": []})),
        };

        if let Some(plugins) = plugins_info.get("data").and_then(|d| d.get("plugins")) {
            if let Some(plugin_names) = plugins.as_array() {
                for plugin_name in plugin_names {
                    if let Some(name) = plugin_name.as_str() {
                        if is_hidden_tool(name) {
                            continue;
                        }

                        // 获取插件的 meta 信息
                        if let Ok(meta) = parent.meta(name) {
                            all_tools.push(json!({
                                "name": name,
                                "description": meta.description
                            }));
                        }
                    }
                }
            }
        }

        // 返回格式优化给 LLM context 注入
        Ok(json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "tools::search_tools",
                        "description": "使用工具列表中的精确名称搜索。示例: keywords=[\"openai\"]",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "keywords": {
                                    "type": "array",
                                    "items": {"type": "string"},
                                    "description": "工具列表中的精确名称，复制 name 字段"
                                }
                            },
                            "required": ["keywords"]
                        }
                    }
                },
                {
                    "type": "object",
                    "description": "可用工具列表。使用方法: 1) 复制精确名称 2) 调用 search_tools 3) 使用 schema",
                    "available_tools": all_tools
                }
            ],
        }))
    }

    /// 根据关键词搜索工具
    /// 返回详细信息包括参数 schema
    pub async fn search_tools(&self, keywords: Vec<String>) -> Result<Value, PluginError> {
        let parent = self.get_parent()
            .ok_or_else(|| PluginError::NotFound("没有父插件引用".to_string()))?;

        let mut matched_tools: Vec<Value> = Vec::new();

        // 获取所有插件列表
        let result = parent.invoke("", json!({}))?;
        let plugins_info = match result {
            crate::core::types::InvokeStream::Single(chunk) => chunk.data,
            _ => return Ok(json!({"matched_tools": []})),
        };

        if let Some(plugins) = plugins_info.get("data").and_then(|d| d.get("plugins")) {
            if let Some(plugin_names) = plugins.as_array() {
                for plugin_name in plugin_names {
                    if let Some(name) = plugin_name.as_str() {
                        if is_hidden_tool(name) {
                            continue;
                        }

                        // 获取插件的 meta 信息
                        if let Ok(meta) = parent.meta(name) {
                            // 检查是否匹配任何关键词
                            let matches = keywords.iter().any(|kw| {
                                let kw_lower = kw.to_lowercase();
                                name.to_lowercase().contains(&kw_lower)
                                    || meta.description.to_lowercase().contains(&kw_lower)
                            });

                            if matches {
                                matched_tools.push(json!({
                                    "full_name": name,
                                    "description": meta.description,
                                    "parameters": meta.input.clone().unwrap_or(json!({})),
                                    "usage": format!("调用 {} 并使用上述参数 schema", name)
                                }));
                            }
                        }
                    }
                }
            }
        }

        Ok(json!({
            "query": keywords,
            "matched_count": matched_tools.len(),
            "matched_tools": matched_tools,
            "usage": "直接使用匹配的工具及其参数 schema"
        }))
    }
}