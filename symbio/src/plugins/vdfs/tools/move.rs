//! `vdfs_move` —— 移动 / 重命名（**面向 LLM：成功 message**）
//!
//! 原生 local 没有对应的 `move` 工具；此处复刻「成功即返回友好 message」的呈现约定，
//! 「访问文件系统」改为调用封装 provider 的 `move_item`。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeResponse, PluginPayload,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// 移动 / 重命名工具（框架原生 `Capability`）
pub struct MoveTool {
    provider: Arc<ToolVdfs>,
}

impl MoveTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for MoveTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_move",
            "移动或重命名文件 / 目录（目标父目录不存在时自动创建）。",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "源路径" },
                    "to": { "type": "string", "description": "目标路径" }
                },
                "required": ["from", "to"]
            }),
            vec!["{\"from\":\"src/a.rs\",\"to\":\"src/b.rs\"}"],
            Some(crate::symbio_core::ToolContextRetention::LastOnly),
        )
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: super::super::protocol::VdfsMoveRequest = request_of(&ctx);
        self.provider.move_item(&ctx, &req.from, &req.to).await?;

        let message = format!("已将 {} 移动 / 重命名为 {}", req.from, req.to);
        Ok(PluginPayload::new(&json!({
            "success": true,
            "from": req.from,
            "to": req.to,
            "message": message,
        })))
    }
}
