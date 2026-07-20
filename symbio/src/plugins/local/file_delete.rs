//! 文件删除工具 - 实现 Capability（对应 Trae 的 DeleteFile）
//!
//! 删除一个或多个文件（file_paths 数组）。仅允许删除工作区范围内的普通文件，
//! 拒绝绝对路径越界、`..` 遍历、符号链接与目录，避免误删。

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

fn ok(payload: Value) -> InvokeResponse<PluginPayload> {
    Ok(PluginPayload::new(&payload))
}

#[derive(Clone)]
pub struct FileDeleteTool {
    security: Arc<SecurityPolicy>,
}

impl FileDeleteTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        let file_paths = args
            .get("file_paths")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                PluginError::ValidationError("缺少 'file_paths' 参数（数组）".to_string())
            })?;

        if file_paths.is_empty() {
            return Err(PluginError::ValidationError(
                "'file_paths' 不能为空".to_string(),
            ));
        }

        let workspace_dir = PathBuf::from(shellexpand::tilde(workdir).to_string());

        let mut deleted = Vec::new();
        let mut failed = Vec::new();

        for fp in file_paths {
            let raw = match fp.as_str() {
                Some(s) => s,
                None => {
                    failed.push(json!({ "path": fp, "error": "非字符串路径" }));
                    continue;
                },
            };

            // 拒绝路径遍历
            if raw.contains("..") {
                failed.push(json!({ "path": raw, "error": "不允许使用路径遍历 '..'" }));
                continue;
            }

            let full = if PathBuf::from(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                workspace_dir.join(raw)
            };

            // 解析符号链接并校验范围
            let resolved = match tokio::fs::canonicalize(&full).await {
                Ok(p) => p,
                Err(e) => {
                    failed.push(json!({ "path": raw, "error": format!("无法解析路径: {e}") }));
                    continue;
                },
            };

            if !self
                .security
                .is_path_allowed_for_read(&resolved, &workspace_dir)
                .await
            {
                failed.push(json!({ "path": raw, "error": "路径超出工作区范围" }));
                continue;
            }

            // 拒绝符号链接（canonicalize 已解析，这里再确认原路径非链接）
            if full.is_symlink() {
                failed.push(json!({ "path": raw, "error": "拒绝删除符号链接" }));
                continue;
            }

            // 仅允许删除普通文件，禁止目录
            if resolved.is_dir() {
                failed.push(json!({ "path": raw, "error": "拒绝删除目录（仅支持文件）" }));
                continue;
            }

            match tokio::fs::remove_file(&resolved).await {
                Ok(_) => deleted.push(raw.to_string()),
                Err(e) => failed.push(json!({ "path": raw, "error": format!("{e}") })),
            }
        }

        Ok(json!({
            "type": "file_deletion",
            "deleted": deleted,
            "failed": failed,
            "message": format!("已删除 {} 个文件，{} 个失败", deleted.len(), failed.len()),
        }))
    }
}

#[async_trait]
impl Capability for FileDeleteTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "delete_file".to_string(),
            description:
                "删除一个或多个文件（file_paths 数组）。仅限工作区范围内普通文件，拒绝越界/符号链接/目录。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_paths": {
                        "type": "array",
                        "description": "要删除的文件路径数组（绝对或相对工作区）",
                        "items": { "type": "string" }
                    }
                },
                "required": ["file_paths"]
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
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
        ok(self.execute_inner(&args, &workdir_str).await?)
    }
}
