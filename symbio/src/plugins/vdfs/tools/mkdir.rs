//! `vdfs_mkdir` —— 新建目录（**面向 LLM：成功 message**）
//!
//! 原生 local 没有对应的 `mkdir` 工具；此处复刻「成功即返回友好 message」的呈现约定，
//! 「访问文件系统」改为调用封装 provider 的 `mkdir`。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeResponse, PluginPayload,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// 新建目录工具（框架原生 `Capability`）
pub struct MkdirTool {
    provider: Arc<ToolVdfs>,
}

impl MkdirTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for MkdirTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_mkdir",
            "新建目录（父目录不存在时自动创建）。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"docs/notes\"}"],
            Some(crate::symbio_core::ToolContextRetention::LastOnly),
        )
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: super::super::protocol::VdfsPathRequest = request_of(&ctx);
        self.provider.mkdir(&ctx, &req.path).await?;

        let message = format!("已创建目录 {}", req.path);
        Ok(PluginPayload::new(&json!({
            "success": true,
            "path": req.path,
            "message": message,
        })))
    }
}
