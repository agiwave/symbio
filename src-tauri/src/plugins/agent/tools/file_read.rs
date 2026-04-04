//! 文件读取工具 - 实现 Plugin trait

use super::policy::SecurityPolicy;
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB

/// 文件读取工具
pub struct FileReadTool {
    security: Arc<SecurityPolicy>,
}

impl FileReadTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "read_file".to_string(),
            description: "读取文件内容，支持行号和分页。相对路径从工作目录开始。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径（相对路径从工作目录开始）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "起始行号（从1开始，默认1）"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "返回的最大行数（默认全部）"
                    }
                },
                "required": ["path"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "文件内容（带行号）" },
                    "path": { "type": "string", "description": "文件路径" },
                    "total_lines": { "type": "integer", "description": "总行数" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: Value) -> Result<Value, PluginError> {
        let path = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 path 参数".to_string()))?;

        // 检查速率限制
        if self.security.is_rate_limited() {
            return Err(PluginError::InternalError("速率限制：操作过于频繁".into()));
        }

        // 检查路径权限
        if !self.security.is_path_allowed_for_read(path).await {
            return Err(PluginError::InternalError(
                format!("路径不允许访问: {}", path)
            ));
        }

        // 获取工作区目录
        let workspace_dir = self.security.get_workspace_dir().await;
        
        // 构建完整路径
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        // 解析符号链接
        let resolved = tokio::fs::canonicalize(&full_path).await
            .map_err(|e| PluginError::InternalError(format!("无法解析路径: {}", e)))?;

        // 再次检查解析后的路径
        let workspace_dir = self.security.get_workspace_dir().await;
        if !resolved.starts_with(&*workspace_dir) && !self.security.allowed_roots.iter().any(|r| resolved.starts_with(r)) {
            return Err(PluginError::InternalError("路径解析后超出允许范围".into()));
        }

        // 检查文件大小
        let metadata = tokio::fs::metadata(&resolved).await
            .map_err(|e| PluginError::InternalError(format!("无法读取文件元数据: {}", e)))?;

        if metadata.len() > MAX_FILE_SIZE {
            return Err(PluginError::InternalError(
                format!("文件过大: {} 字节 (限制: {} 字节)", metadata.len(), MAX_FILE_SIZE)
            ));
        }

        // 记录动作
        self.security.record_action();

        // 读取文件
        let content = tokio::fs::read_to_string(&resolved).await
            .map_err(|e| PluginError::InternalError(format!("读取文件失败: {}", e)))?;

        // 添加行号
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let offset = args.get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v.max(1) as usize).saturating_sub(1))
            .unwrap_or(0);

        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);

        let start = offset.min(total);
        let end = limit.map(|l| (start + l).min(total)).unwrap_or(total);

        let numbered: String = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        let summary = if start > 0 || end < total {
            format!("\n[行 {}-{}，共 {} 行]", start + 1, end, total)
        } else {
            format!("\n[共 {} 行]", total)
        };

        Ok(json!({
            "content": format!("{}{}", numbered, summary),
            "path": path,
            "total_lines": total
        }))
    }
}

#[async_trait]
impl Plugin for FileReadTool {
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        // 这个工具没有子路径
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