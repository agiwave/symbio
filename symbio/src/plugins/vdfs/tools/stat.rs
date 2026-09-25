//! `vdfs_stat` —— 读取单个节点的元数据（名称 / 访问位 / 大小 / 更新时间 / 类型）。
//!
//! 无面向 LLM 的专属封装：把封装 provider 的 `stat` 结果（`VdfsNode`）透传给大模型。

use super::{request_of, tool, ToolVdfs};
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 读取元数据工具（框架原生 `Capability`）
pub struct StatTool {
    provider: Arc<ToolVdfs>,
}

impl StatTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for StatTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_stat",
            "读取单个节点的元数据（名称/访问位/大小/更新时间/类型），不返回内容。",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_DESC }
                },
                "required": ["path"]
            }),
            vec!["{\"path\":\"README.md\"}"],
            None,
        )
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let req: super::super::protocol::VdfsPathRequest = request_of(&args);
        let node = self.provider.stat(&ctx, &req.path).await?;
        Ok(serde_json::to_value(&node)?)
    }
}
