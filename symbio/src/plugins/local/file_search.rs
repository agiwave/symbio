//! 文件搜索工具 - 使用 Glob 模式 - 实现 Tool trait

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_RESULTS: usize = 1000;

/// Glob 搜索工具
#[derive(Clone)]
pub struct FileSearchTool {
    security: Arc<SecurityPolicy>,
}

impl FileSearchTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 'pattern' 参数".to_string()))?;

        // 安全检查：拒绝绝对路径
        if pattern.starts_with('/') || pattern.starts_with('\\') {
            return Err(PluginError::ValidationError(
                "不允许使用绝对路径。请使用相对 Glob 模式。".to_string(),
            ));
        }

        // 安全检查：拒绝路径遍历
        if pattern.contains("../") || pattern.contains("..\\") || pattern == ".." {
            return Err(PluginError::ValidationError(
                "不允许在 Glob 模式中使用路径遍历 ('..')。".to_string(),
            ));
        }

        // 构建完整模式
        let workspace_dir = std::path::PathBuf::from(shellexpand::tilde(workdir).to_string());
        let workspace_canon = tokio::fs::canonicalize(&workspace_dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("无法解析工作区目录: {e}")))?;

        // 可选搜索基目录：相对工作区，拒绝绝对路径与 '..' 遍历
        let search_base = if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
            if p.starts_with('/') || p.starts_with('\\') || p.contains("..") {
                return Err(PluginError::ValidationError(
                    "path 不允许使用绝对路径或 '..' 遍历。".to_string(),
                ));
            }
            let base = workspace_dir.join(p);
            let base_canon = tokio::fs::canonicalize(&base)
                .await
                .map_err(|e| PluginError::InternalError(format!("无法解析搜索基目录: {e}")))?;
            if !self
                .security
                .is_path_allowed_for_read(&base_canon, &workspace_dir)
                .await
            {
                return Err(PluginError::ValidationError(
                    "搜索基目录超出工作区范围。".to_string(),
                ));
            }
            base_canon
        } else {
            workspace_canon.clone()
        };
        let full_pattern = search_base.join(pattern).to_string_lossy().to_string();

        let entries = match glob::glob(&full_pattern) {
            Ok(paths) => paths,
            Err(e) => {
                return Err(PluginError::ValidationError(format!(
                    "无效的 Glob 模式: {e}"
                )))
            }
        };

        let mut results = Vec::new();
        let mut truncated = false;

        for entry in entries {
            let path = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };

            let resolved = match tokio::fs::canonicalize(&path).await {
                Ok(p) => p,
                Err(_) => continue,
            };

            // 使用统一的路径验证方法
            if !self
                .security
                .is_path_allowed_for_read(&resolved, &workspace_dir)
                .await
            {
                continue;
            }

            if resolved.is_dir() {
                continue;
            }

            // 直接使用原始的带 \\?\ 前缀的路径进行 strip_prefix（两者都带前缀，可以正确匹配）
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
            format!("未找到匹配模式 '{pattern}' 的文件。")
        } else {
            let mut msg = results.join("\n");
            if truncated {
                msg.push_str(&format!("\n\n[结果已截断：显示前 {MAX_RESULTS} 个匹配]"));
            }
            msg.push_str(&format!("\n\n总计: {} 个文件", results.len()));
            msg
        };

        Ok(json!({
            "results": results,
            "truncated": truncated,
            "message": message
        }))
    }
}

#[async_trait]
impl Capability for FileSearchTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "glob_search".to_string(),
            description: "文件名模式搜索。使用 Glob 模式 matching 文件名。".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob 模式，如 **/*.rs"
                    },
                    "path": {
                        "type": "string",
                        "description": "可选搜索基目录（相对工作区，限定子目录范围，对齐 Trae Glob 的 path）"
                    }
                },
                "required": ["pattern"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "pattern='**/*.rs'".to_string(),
                "pattern='src/**/*.toml'".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let workdir_str = ctx.get(crate::symbio_core::WORKDIR).ok_or_else(|| {
            PluginError::ValidationError("Missing workdir in context".to_string())
        })?;
        if workdir_str.is_empty() {
            return Err(PluginError::ValidationError(
                "Empty workdir in context".to_string(),
            ));
        }
        let data = self.execute_inner(&args, &workdir_str).await?;
        Ok(PluginPayload::new(&data))
    }
}
