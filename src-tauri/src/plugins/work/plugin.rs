//! Work 插件 - 工作区管理

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};

pub struct WorkPlugin {
    meta: PluginMeta,
}

impl WorkPlugin {
    pub fn new() -> Self {
        WorkPlugin {
            meta: PluginMeta {
                name: "work".to_string(),
                description: "工作区管理插件".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "create", "get", "update", "delete"],
                            "description": "操作类型"
                        },
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "content": { "type": "string" },
                        "parentId": { "type": "string" }
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
            },
        }
    }
}

impl Default for WorkPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Plugin for WorkPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("list");

        match action {
            "list" => Ok(InvokeStream::single(json!({
                "success": true,
                "data": {
                    "documents": [
                        {"id": "doc-1", "title": "示例文档 1", "parentId": null},
                        {"id": "doc-2", "title": "示例文档 2", "parentId": null},
                    ]
                }
            }))),
            "create" => {
                let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("新文档");
                let parent_id = input.get("parentId").and_then(|v| v.as_str());
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {
                        "id": format!("doc-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()),
                        "title": title,
                        "parentId": parent_id,
                        "content": ""
                    },
                    "message": "文档创建成功"
                })))
            }
            "get" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"id": id, "title": "示例文档", "parentId": null, "content": "# 示例文档\n\n这是一个示例文档内容。"}
                })))
            }
            "update" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"id": id},
                    "message": "文档更新成功"
                })))
            }
            "delete" => {
                let id = input.get("id").and_then(|v| v.as_str())
                    .ok_or_else(|| PluginError::ValidationError("缺少 id 参数".to_string()))?;
                Ok(InvokeStream::single(json!({
                    "success": true,
                    "data": {"id": id},
                    "message": "文档删除成功"
                })))
            }
            _ => Err(PluginError::ValidationError(format!("未知操作: {}", action))),
        }
    }
}
