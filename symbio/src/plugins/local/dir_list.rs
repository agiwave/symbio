//! 目录列举工具 - 实现 Tool trait（等价于 Trae 的 LS）
//!
//! 安全地列举工作区内的目录内容，支持按名称忽略（glob）。
//! 仅做只读列举，不修改任何文件。

use super::policy::SecurityPolicy;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_ENTRIES: usize = 2000;

/// 目录列举工具
#[derive(Clone)]
pub struct DirListTool {
    security: Arc<SecurityPolicy>,
}

impl DirListTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(&self, args: &Value, workdir: &str) -> InvokeResponse<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ignore: Vec<String> = args
            .get("ignore")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // 安全检查：拒绝路径遍历
        if path.contains("..") {
            return Err(PluginError::ValidationError(
                "不允许在路径中使用 '..' 进行目录遍历。".to_string(),
            ));
        }

        let workspace_dir = PathBuf::from(shellexpand::tilde(workdir).to_string());

        // 解析目标目录：空路径或 "." 表示工作区根
        let target = if path.is_empty() || path == "." {
            workspace_dir.clone()
        } else if PathBuf::from(path).is_absolute() {
            PathBuf::from(path)
        } else {
            workspace_dir.join(path)
        };

        // 目标必须存在且为目录
        let meta = tokio::fs::metadata(&target).await.map_err(|e| {
            PluginError::ValidationError(format!("无法访问路径 '{path}': {e}"))
        })?;
        if !meta.is_dir() {
            return Err(PluginError::ValidationError(format!(
                "路径 '{path}' 不是一个目录。"
            )));
        }

        let target_canon = tokio::fs::canonicalize(&target)
            .await
            .map_err(|e| PluginError::InternalError(format!("解析目录失败: {e}")))?;

        // 安全校验：必须在工作区内
        if !self
            .security
            .is_path_allowed_for_read(&target_canon, &workspace_dir)
            .await
        {
            return Err(PluginError::ValidationError(format!(
                "路径超出工作区: {}",
                target_canon.display()
            )));
        }

        let ignore_globs: Vec<glob::Pattern> = ignore
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();

        let mut entries: Vec<Value> = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&target_canon)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取目录失败: {e}")))?;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|e| PluginError::InternalError(format!("遍历目录失败: {e}")))?
        {
            let name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if ignore_globs.iter().any(|g| g.matches(&name)) {
                continue;
            }
            let emeta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_dir = emeta.is_dir();
            let size = if is_dir { 0 } else { emeta.len() };
            let modified = emeta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.push(json!({
                "name": name,
                "type": if is_dir { "directory" } else { "file" },
                "size": size,
                "modified": modified,
            }));
            if entries.len() >= MAX_ENTRIES {
                break;
            }
        }

        // 目录在前、文件在后，各自按名称排序
        entries.sort_by(|a, b| {
            let ta = a["type"].as_str().unwrap_or("");
            let tb = b["type"].as_str().unwrap_or("");
            if ta != tb {
                return tb.cmp(ta); // directory 优先
            }
            let na = a["name"].as_str().unwrap_or("");
            let nb = b["name"].as_str().unwrap_or("");
            na.cmp(nb)
        });

        let truncated = entries.len() >= MAX_ENTRIES;
        let message = if entries.is_empty() {
            format!("目录 '{path}' 为空或无可列举条目。")
        } else {
            format!("已列举 {} 个条目。", entries.len())
        };

        Ok(json!({
            "entries": entries,
            "truncated": truncated,
            "count": entries.len(),
            "message": message,
        }))
    }
}

#[async_trait]
impl Capability for DirListTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "dir_list".to_string(),
            description:
                "列举给定目录下的文件与子目录。path 缺省为工作区根目录；支持 ignore 忽略指定名称。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要列举的目录路径（相对工作区或绝对路径），缺省为工作区根"
                    },
                    "ignore": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要忽略的文件/目录名称模式（glob，如 ['*.tmp', 'node_modules']）"
                    }
                },
                "required": []
            }),
            category: Some(crate::symbio_core::CapabilityCategory::FileOperation),
            examples: Some(vec![
                "path='src'".to_string(),
                "path='.', ignore=['target', 'node_modules']".to_string(),
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
