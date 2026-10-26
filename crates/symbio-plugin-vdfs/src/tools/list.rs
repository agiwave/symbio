//! `vdfs_list` —— 列出目录下的直接子项（**面向 LLM：ignore 过滤 + 原生 dir_list 形状**）
//!
//! 移植自已下线的原生 `dir_list`：原生 `dir_list` 直接 `tokio::fs::read_dir` 遍历工作区；
//! 本工具把「访问文件系统」改为「调用封装 provider 的 `list`」，拿到条目列表
//! （[`VdfsItem`] = 地址 + `VdfsNode`）后做**与原生 `dir_list` 一致的 ignore 过滤**，
//! 封装成 `{entries:[{name,type,size,modified}], truncated, count, message}` 形状。
//!
//! 条目地址在本工具里用不上（只报 `name`），所以只取 `item.node`。
//!
//! `VdfsItem`
//!
//! **虚拟目录 `.vdfsv2`**：系统资源类别统一挂接在此目录之下；对 `.vdfsv2` 列目录
//! 即返回当前可访问的全部类别，无需独立工具。

use super::{tool, ToolVdfs};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_core::{
    Capability, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError, PluginInvokeRequest,
};

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
            vec!["{\"path\":\".\",\"ignore\":[\"target\",\"node_modules\"]}", "{\"path\":\"src\"}", "{\"path\":\".vdfsv2\"}"],
            CapabilityRiskLevel::Low,
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

        // 目录在前、文件在后，各自按名称排序（与 provider 的 list 同一口径）。
        // ⚠️ 判据是 `type` 字符串的字典序的反面：这里**显式比布尔**，不靠
        // "directory" < "file" 这种字面量巧合——一旦呈现形状改名，排序会静默反向。
        let is_dir = |e: &Value| e["type"].as_str() == Some("directory");
        entries.sort_by(|a, b| {
            is_dir(b).cmp(&is_dir(a)).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            })
        });

        let truncated = entries.len() >= MAX_ENTRIES;
        // 工作目录根在回执里印成 `.`——`/` 已经是文件系统的绝对锚点，
        // 把空地址显示成 `/` 会让模型下一次照着它去列磁盘根（见 D-172）。
        let shown = if path.is_empty() { "." } else { path.as_str() };
        // 回执自带账目：条目数按类型分开报。弱模型只会照抄它读到的那一段，
        // 「28 个条目」写在 message 里它就知道自己该说几个；只给一个总数时，
        // 它数了前 17 行便宣布其余的「是第二层」（实测会话 cli1a175d1ea440）。
        let dirs = entries.iter().filter(|e| is_dir(e)).count();
        let files = entries.len() - dirs;
        let message = if entries.is_empty() {
            format!("目录 '{shown}' 为空或无可列举条目。")
        } else {
            format!(
                "已列举 {} 个条目（{dirs} 个目录 + {files} 个文件），全部在第一层。",
                entries.len()
            )
        };

        Ok(json!({
            "entries": entries,
            "truncated": truncated,
            "count": entries.len(),
            "message": message,
        }))
    }
}

#[cfg(test)]
#[path = "list.test.rs"]
mod tests;
