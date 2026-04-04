//! 文件编辑工具 - 实现 Plugin trait

use super::policy::SecurityPolicy;
use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// 文件编辑工具
pub struct FileEditTool {
    security: Arc<SecurityPolicy>,
}

impl FileEditTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "file_edit".to_string(),
            description: "编辑文件（精确字符串替换）。old_string 必须在文件中精确匹配一次。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "文件路径"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "要查找的文本（必须精确匹配一次）"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "替换为的文本（默认空字符串）"
                    }
                },
                "required": ["path", "old_string"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "message": { "type": "string" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: &Value) -> Result<StreamChunk, PluginError> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'path' 参数".to_string()))?;
        
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'old_string' 参数".to_string()))?;
        
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 验证 old_string 不为空
        if old_string.is_empty() {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("old_string 不能为空".to_string()),
            });
        }

        // 安全检查：禁止 null 字节
        if path.contains('\0') {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("无效路径：不允许 null 字节".to_string()),
            });
        }

        // 构建完整路径
        let workspace_dir = self.security.get_workspace_dir().await;
        let full_path = if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };
        drop(workspace_dir);

        // 获取父目录
        let parent = full_path.parent().ok_or_else(|| {
            PluginError::ValidationError("无效路径：缺少父目录".to_string())
        })?;

        // 规范化父目录以进行验证（处理 Windows 的 \\?\ 前缀问题）
        let normalized_parent = if parent.exists() {
            // 如果存在，使用 canonicalize 获取规范路径
            tokio::fs::canonicalize(parent).await
                .map_err(|e| PluginError::InternalError(format!("解析路径失败: {}", e)))?
        } else {
            // 如果不存在（新文件），直接使用原始路径
            parent.to_path_buf()
        };

        // 验证解析后的路径
        if !self.security.is_path_allowed_for_write(&normalized_parent).await {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(format!(
                    "路径超出工作区: {}",
                    normalized_parent.display()
                )),
            });
        }

        // 获取文件名
        let _file_name = full_path.file_name().ok_or_else(|| {
            PluginError::ValidationError("无效路径：缺少文件名".to_string())
        })?;

        // 使用原始的 full_path 进行文件操作
        let resolved_target = full_path.clone();

        // 符号链接检查
        if let Ok(meta) = tokio::fs::symlink_metadata(&resolved_target).await {
            if meta.file_type().is_symlink() {
                return Ok(StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!(
                        "拒绝编辑符号链接: {}",
                        resolved_target.display()
                    )),
                });
            }
        }

        // 读取文件
        let content = tokio::fs::read_to_string(&resolved_target)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取文件失败: {}", e)))?;

        // 统计匹配次数
        let match_count = content.matches(old_string).count();
        if match_count == 0 {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(format!("未找到 old_string 在文件 '{}' 中", path)),
            });
        }
        if match_count > 1 {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some(format!(
                    "old_string 在文件 '{}' 中匹配 {} 次；必须精确匹配一次",
                    path, match_count
                )),
            });
        }

        // 执行替换
        let new_content = content.replacen(old_string, new_string, 1);

        // 写入结果
        tokio::fs::write(&resolved_target, &new_content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入文件失败: {}", e)))?;

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "message": format!("已编辑 {}: 替换了 1 处 ({} 字节)", path, new_content.len())
            }),
            done: true,
            error: None,
        })
    }
}

#[async_trait]
impl Plugin for FileEditTool {
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
                self.execute_inner(&input).await
            })
        })?;

        Ok(InvokeStream::Single(result))
    }
}