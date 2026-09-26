//! `vdfs_read` —— 读取文件内容（**面向 LLM：带行号 + 分页**）
//!
//! 移植自已下线的原生 `read_file`：原生 `read_file` 直接 `tokio::fs::read_to_string`
//! 再做行号加工；本工具把「访问文件系统」改为「调用封装 provider 的 `read`」，
//! 拿到 `VdfsContent` 后做**与原生 `read_file` 完全一致的行号 + 分页封装**。
//! 原生 `read_file` 对图片返回 base64 多模态载荷，本工具对 `VdfsContent.binary`
//! 直接透传（已含 `b64` / `mime`，供多模态链路消费）。

use super::{tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 读文件工具（框架原生 `Capability`）
pub struct ReadTool {
    provider: Arc<ToolVdfs>,
}

impl ReadTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for ReadTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_read",
            "读取文件内容（与原生 read_file 一致：带行号、支持分页）。文本文件返回 content 字段（含行号与 [行 X-Y，共 N 行] 摘要）；二进制文件（如图片）返回 base64 的 b64 字段且 binary=true。目录不可读，请用 vdfs_list。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "offset": { "type": "integer", "description": "起始行号（从 1 开始，默认 1）" },
                    "limit": { "type": "integer", "description": "返回的最大行数（默认全部）" }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"README.md\"}", "{\"path\":\"src/main.rs\",\"limit\":50}"],
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
        let content = self.provider.read(&ctx, &path).await?;

        // 二进制（图片等）原样透传：VdfsContent 已含 b64 + mime，由多模态链路消费
        if content.binary {
            return Ok(serde_json::to_value(&content)?);
        }
        let Some(text) = content.text else {
            return Ok(serde_json::to_value(&content)?);
        };

        // 以下逻辑与原生 read_file 的 execute_inner 完全一致（行号 + 分页）
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();

        let offset = raw
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v.max(1) as usize).saturating_sub(1))
            .unwrap_or(0);
        let limit = raw
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
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
            format!("\n[共 {total} 行]")
        };

        Ok(json!({
            "content": format!("{numbered}{summary}"),
            // 回显的是**请求地址**：内容不带地址（内容总是「这个节点」的内容）
            "path": path,
            "total_lines": total,
        }))
    }
}
