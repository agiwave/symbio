//! 文件写入工具 - 实现 Plugin trait

use super::policy::SecurityPolicy;
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// 文件写入工具
pub struct FileWriteTool {
    security: Arc<SecurityPolicy>,
}

impl FileWriteTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "write_file".to_string(),
            description: "写入文件内容。相对路径从工作目录开始。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "文件内容"
                    }
                },
                "required": ["path", "content"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "path": { "type": "string" },
                    "bytes_written": { "type": "integer" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: Value) -> Result<Value, PluginError> {
        let path = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;

        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 content 参数".to_string()))?;

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 检查路径权限
        if !self.security.is_path_allowed_for_write(path) {
            return Err(PluginError::InternalError(
                format!("路径不允许写入: {}", path)
            ));
        }

        // 构建完整路径
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.security.workspace_dir.join(path)
        };

        // 确保父目录存在
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| PluginError::InternalError(format!("创建目录失败: {}", e)))?;
        }

        // 记录动作
        self.security.record_action();

        // 写入文件
        tokio::fs::write(&full_path, content).await
            .map_err(|e| PluginError::InternalError(format!("写入文件失败: {}", e)))?;

        Ok(json!({
            "success": true,
            "path": path,
            "bytes_written": content.len()
        }))
    }
}

#[async_trait]
impl Plugin for FileWriteTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(Self::create_meta())
        } else {
            Err(PluginError::NotFound(format!("路径不存在: {}", path)))
        }
    }

    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("路径不存在: {}", path)));
        }

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match self.execute_inner(input).await {
                    Ok(result) => StreamChunk {
                        data: result,
                        done: true,
                        error: None,
                    },
                    Err(e) => StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(e.to_string()),
                    },
                }
            })
        });

        Ok(InvokeStream::Single(result))
    }
}