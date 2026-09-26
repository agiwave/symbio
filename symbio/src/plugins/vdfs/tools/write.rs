//! `vdfs_write` —— 写入文件（**面向 LLM：成功 message**）
//!
//! 移植自已下线的原生 `write_file`：原生 `write_file` 直接 `tokio::fs::write`；本工具
//! 把「访问文件系统」改为「调用封装 provider 的 `write`」，拿到 `VdfsWriteResponse`
//! 后封装成原生 `write_file` 的 `{success, path, created}` 形状。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 写入文件工具（框架原生 `Capability`）
pub struct WriteTool {
    provider: Arc<ToolVdfs>,
}

impl WriteTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for WriteTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_write",
            "写入（创建或覆盖）文件内容。父目录不存在时自动创建。写前应先用 vdfs_read 或 vdfs_stat 确认目标存在性与访问位；失败时返回的错误可能含字段级校验信息，请据其修正后重试。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "text": { "type": "string", "description": "文本内容（与 b64 二选一）" },
                    "b64": { "type": "string", "description": "base64 内容（与 text 二选一）" }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"notes.md\",\"text\":\"# 标题\\n\"}"],
            Some(crate::symbio_core::CapabilityToolContextRetention::LastOnly),
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        super::ensure_required(&args, "path")?;
        let req: super::super::protocol::VdfsWriteRequest = request_of(&args);
        let content = req.to_content();
        let data = self.provider.write(&ctx, &req.path, &content).await?;

        let message = if data.created {
            format!("已创建文件 {}", req.path)
        } else {
            format!("已覆盖文件 {}", req.path)
        };
        Ok(json!({
            "success": true,
            "path": req.path,
            "created": data.created,
            "message": message,
        }))
    }
}
