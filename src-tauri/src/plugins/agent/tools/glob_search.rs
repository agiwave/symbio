//! 文件搜索工具 - 使用 Glob 模式

use crate::core::types::{PluginError, StreamChunk};
use serde_json::{Value, json};
use std::path::PathBuf;

const MAX_RESULTS: usize = 1000;

/// Glob 搜索工具
pub struct GlobSearchTool;

impl GlobSearchTool {
    pub fn new() -> Self {
        Self
    }

    /// 执行 Glob 搜索
    pub async fn execute(
        &self,
        args: &Value,
        workspace_dir: &PathBuf,
    ) -> Result<StreamChunk, PluginError> {
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
        let full_pattern = workspace_dir.join(pattern).to_string_lossy().to_string();

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

        let workspace_canon = std::fs::canonicalize(workspace_dir)
            .map_err(|e| PluginError::InternalError(format!("无法解析工作区目录: {}", e)))?;

        let mut results = Vec::new();
        let mut truncated = false;

        for entry in entries {
            let path = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };

            // 解析符号链接并验证仍在工作区内
            let resolved = match std::fs::canonicalize(&path) {
                Ok(p) => p,
                Err(_) => continue,
            };

            // 检查路径是否在工作区内
            if !resolved.starts_with(&workspace_canon) {
                continue;
            }

            // 只包含文件，不包含目录
            if resolved.is_dir() {
                continue;
            }

            // 转换为相对于工作区的路径
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

impl Default for GlobSearchTool {
    fn default() -> Self {
        Self::new()
    }
}
