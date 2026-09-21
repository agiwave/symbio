//! `vdfs_edit` —— 编辑文件（精确字符串替换，**面向 LLM：成功 message**）
//!
//! 移植自已下线的原生 `file_edit`：原生 `file_edit` 直接 `tokio::fs::read_to_string` +
//! `write` 做精确替换；本工具把「访问文件系统」改为「调用封装 provider 的 `edit`」，
//! 拿到 `VdfsEditResponse` 后封装成原生 `file_edit` 的 `{success, message}` 形状。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, InvokeRequest, PluginError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 编辑文件工具（框架原生 `Capability`）
pub struct EditTool {
    provider: Arc<ToolVdfs>,
}

impl EditTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for EditTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_edit",
            "编辑文件（精确字符串替换，与原生 file_edit 一致）。old_string 必须在文件中精确匹配一次；自动保持原换行符风格（CRLF/LF）；拒绝编辑符号链接。匹配 0 次或多次均报错。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "old_string": { "type": "string", "description": "要查找并替换的文本（必须精确匹配一次）" },
                    "new_string": { "type": "string", "description": "替换为的文本（默认空字符串）" }
                },
                "required": ["path", "old_string"]
            }),
            vec!["{\"path\":\"src/main.rs\",\"old_string\":\"fn old()\",\"new_string\":\"fn new()\"}"],
            Some(crate::symbio_core::ToolContextRetention::LastOnly),
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn InvokeRequest>,
    ) -> Result<Value, PluginError> {
        let req: super::super::protocol::VdfsEditRequest = request_of(&args);
        let data = self
            .provider
            .edit(&ctx, &req.path, &req.old_string, &req.new_string)
            .await?;

        // provider 已给出可读说明（如「内容已为最新」）时优先采用；否则据 replaced 生成
        let message = match data.message {
            Some(m) if !m.is_empty() => m,
            _ if data.replaced == 0 => format!("已编辑 {}: 内容已为最新，无需修改", req.path),
            _ => format!("已编辑 {}: 替换了 {} 处", req.path, data.replaced),
        };

        Ok(json!({
            "success": true,
            "message": message,
        }))
    }
}
