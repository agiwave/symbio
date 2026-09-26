//! `vdfs_list` —— 列出目录下的直接子项（**面向 LLM：ignore 过滤 + 原生 dir_list 形状**）
//!
//! 移植自已下线的原生 `dir_list`：原生 `dir_list` 直接 `tokio::fs::read_dir` 遍历工作区；
//! 本工具把「访问文件系统」改为「调用封装 provider 的 `list`」，拿到条目列表
//! （[`VdfsItem`] = 地址 + `VdfsNode`）后做**与原生 `dir_list` 一致的 ignore 过滤**，
//! 封装成 `{entries:[{name,type,size,modified}], truncated, count, message}` 形状。
//!
//! 条目地址在本工具里用不上（只报 `name`），所以只取 `item.node`。
//!
//! [`VdfsItem`]: crate::symbio_core::VdfsItem
//!
//! **虚拟目录 `.vdfsv2`**：系统资源类别统一挂接在此目录之下；对 `.vdfsv2` 列目录
//! 即返回当前可访问的全部类别，无需独立工具。

use super::{tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_ENTRIES: usize = 2000;

/// 列目录工具（框架原生 `Capability`）
pub struct ListTool {
    provider: Arc<ToolVdfs>,
}

impl ListTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for ListTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_list",
            "列出目录下的直接子项（文件与子目录）。每项返回 name / type（directory|file）/ size / modified；支持 ignore 按名称 glob 过滤（如 ['*.tmp','node_modules']）。与原生 dir_list 一致。系统资源类别挂接在虚拟目录 '.vdfsv2' 下——列 '.vdfsv2' 可枚举当前可访问的全部类别（如 plugin_manager/session 等），列 '.vdfsv2/<类别>/...' 深入具体资源。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "ignore": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要忽略的文件/目录名称模式（glob，如 ['*.tmp', 'node_modules']）"
                    }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"/\"}", "{\"path\":\".vdfsv2\"}", "{\"path\":\".\",\"ignore\":[\"target\",\"node_modules\"]}"],
            None,
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let raw = args;
        super::ensure_required(&raw, "path")?;
        let path = raw
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let items = self.provider.list(&ctx, &path).await?;

        // 与原生 dir_list 一致的 ignore 过滤（按文件名 glob）
        let ignore: Vec<String> = raw
            .get("ignore")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let ignore_globs: Vec<glob::Pattern> = ignore
            .iter()
            .filter_map(|pat| glob::Pattern::new(pat).ok())
            .collect();

        let mut entries: Vec<Value> = Vec::new();
        for it in items {
            let n = &it.node;
            if ignore_globs.iter().any(|g| g.matches(&n.name)) {
                continue;
            }
            entries.push(json!({
                "name": n.name,
                "type": if n.is_dir() { "directory" } else { "file" },
                "size": n.size.unwrap_or(0),
                "modified": n.updated_at.unwrap_or(0),
            }));
            if entries.len() >= MAX_ENTRIES {
                break;
            }
        }

        // 目录在前、文件在后，各自按名称排序（与原生 dir_list 一致）
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
        let shown = if path.is_empty() { "/" } else { path.as_str() };
        let message = if entries.is_empty() {
            format!("目录 '{shown}' 为空或无可列举条目。")
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
