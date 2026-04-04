//! 文件搜索工具 - 使用 Glob 模式 - 实现 Plugin trait

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginError, PluginResult, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_RESULTS: usize = 1000;

/// Glob 搜索工具
pub struct GlobSearchTool {
    workspace_dir: Arc<RwLock<PathBuf>>,
}

impl GlobSearchTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir: Arc::new(RwLock::new(workspace_dir)) }
    }

    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "glob_search".to_string(),
            description: "文件名模式搜索。使用 Glob 模式匹配文件名。".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob 模式，如 **/*.rs"
                    }
                },
                "required": ["pattern"]
            })),
            output: Some(json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "results": { "type": "array" },
                    "truncated": { "type": "boolean" }
                }
            })),
            author: Some("Symbio Team".to_string()),
        }
    }

    async fn execute_inner(&self, args: &Value) -> Result<StreamChunk, PluginError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'pattern' 参数".to_string()))?;

        // 安全检查：拒绝绝对路径
        if pattern.starts_with('/') || pattern.starts_with('\\') {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("不允许使用绝对路径。请使用相对 Glob 模式。".to_string()),
            });
        }

        // 安全检查：拒绝路径遍历
        if pattern.contains("../") || pattern.contains("..\\") || pattern == ".." {
            return Ok(StreamChunk {
                data: json!({}),
                done: true,
                error: Some("不允许在 Glob 模式中使用路径遍历 ('..')。".to_string()),
            });
        }

        // 构建完整模式
        let workspace_dir = self.workspace_dir.read().await;
        let full_pattern = workspace_dir.join(pattern).to_string_lossy().to_string();
        let workspace_canon = std::fs::canonicalize(&*workspace_dir)
            .map_err(|e| PluginError::InternalError(format!("无法解析工作区目录: {}", e)))?;
        drop(workspace_dir);

        let entries = match glob::glob(&full_pattern) {
            Ok(paths) => paths,
            Err(e) => {
                return Ok(StreamChunk {
                    data: json!({}),
                    done: true,
                    error: Some(format!("无效的 Glob 模式: {}", e)),
                });
            }
        };

        let mut results = Vec::new();
        let mut truncated = false;

        for entry in entries {
            let path = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };

            let resolved = match std::fs::canonicalize(&path) {
                Ok(p) => p,
                Err(_) => continue,
            };

            if !resolved.starts_with(&workspace_canon) {
                continue;
            }

            if resolved.is_dir() {
                continue;
            }

            if let Ok(rel) = resolved.strip_prefix(&workspace_canon) {
                results.push(rel.to_string_lossy().to_string());
            }

            if results.len() >= MAX_RESULTS {
                truncated = true;
                break;
            }
        }

        results.sort();

        let message = if results.is_empty() {
            format!("未找到匹配模式 '{}' 的文件。", pattern)
        } else {
            let mut msg = results.join("\n");
            if truncated {
                msg.push_str(&format!("\n\n[结果已截断：显示前 {} 个匹配]", MAX_RESULTS));
            }
            msg.push_str(&format!("\n\n总计: {} 个文件", results.len()));
            msg
        };

        Ok(StreamChunk {
            data: json!({
                "success": true,
                "results": results,
                "truncated": truncated,
                "message": message
            }),
            done: true,
            error: None,
        })
    }
}

#[async_trait]
impl Plugin for GlobSearchTool {
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