//! `vdfs_delete` —— 删除文件或目录（**面向 LLM：成功 message**）
//!
//! 复刻原生 `delete_file` 的「成功即返回友好 message」呈现；「访问文件系统」改为
//! 调用封装 provider 的 `delete`。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeResponse, PluginPayload,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// 删除工具（框架原生 `Capability`）
pub struct DeleteTool {
    provider: Arc<ToolVdfs>,
}

impl DeleteTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for DeleteTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_delete",
            "删除文件或目录。删除目录需 recursive=true。不可逆，删除前务必确认路径。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC },
                    "recursive": { "type": "boolean", "description": "删除目录时是否递归" }
                },
                "required": ["path"]
            }),
            vec![
                "{\"path\":\"tmp/old.txt\"}",
                "{\"path\":\"tmp\",\"recursive\":true}",
            ],
            Some(crate::symbio_core::ToolContextRetention::LastOnly),
        )
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let req: super::super::protocol::VdfsPathRequest = request_of(&ctx);
        self.provider.delete(&ctx, &req.path, req.recursive).await?;

        let message = format!("已删除 {}", req.path);
        Ok(PluginPayload::new(&json!({
            "success": true,
            "path": req.path,
            "message": message,
        })))
    }
}
