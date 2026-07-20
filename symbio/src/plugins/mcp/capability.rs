//! MCP 工具的 `Capability` 包装
//!
//! 每个远程 MCP 工具对应一个 `McpToolCapability` 实例；实例由
//! `McpPlugin::traverse` 在每次 `parent.traverse` 时根据 `manager.discover_tools`
//! 的返回值动态构造并注册到 `tool_manager`。
//!
//! ## 命名
//!
//! 工具名采用 `mcp.<server_name>.<tool_name>` 三段式，避免不同 server
//! 之间的同名工具冲突（例如两个 server 都暴露 `search` 工具）。
//!
//! ## 生命周期
//!
//! - 创建：每次 `traverse` 时构造
//! - 销毁：随 `tool_manager`（通常是 `DefaultToolManager`）一起被丢弃
//! - **不持有**任何 stdio 进程 / http 连接（按需 lazy 加载）

use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
use crate::symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, InvokeRequest, InvokeRequestExt,
    InvokeResponse, PluginError, PluginPayload,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use super::manager::McpManager;
use super::types::McpTool;

/// 单个 MCP 工具的 `Capability` 包装
pub struct McpToolCapability {
    /// MCP server 名称（在 `McpConfig.servers` 中的 key）
    server_name: String,
    /// 远程工具名称
    tool_name: String,
    /// 工具描述
    description: String,
    /// 输入参数 JSON Schema
    input_schema: Value,
    /// 共享的 `McpServerConfig`（在 `traverse` 时克隆传入）
    server_config: McpServerConfig,
    /// 共享的 `McpManager`（无状态，clone 廉价）
    manager: Arc<McpManager>,
}

impl McpToolCapability {
    /// 构造一个 `McpToolCapability`
    pub fn new(
        server_name: String,
        tool: McpTool,
        server_config: McpServerConfig,
        manager: Arc<McpManager>,
    ) -> Self {
        Self {
            server_name,
            tool_name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
            server_config,
            manager,
        }
    }

    /// 三段式工具名：`mcp.<server_name>.<tool_name>`
    pub fn namespaced_name(server_name: &str, tool_name: &str) -> String {
        format!("mcp.{server_name}.{tool_name}")
    }
}

#[async_trait]
impl Capability for McpToolCapability {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: Self::namespaced_name(&self.server_name, &self.tool_name),
            description: format!("[MCP:{}] {}", self.server_name, self.description),
            input_schema: self.input_schema.clone(),
            category: Some(CapabilityCategory::Mcp),
            examples: None,
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let args: Value = ctx.payload()?;
        let result = self
            .manager
            .call_tool(
                &self.server_name,
                &self.server_config,
                &self.tool_name,
                args,
            )
            .await
            .map_err(PluginError::InternalError)?;
        Ok(PluginPayload::new(&result))
    }
}

#[cfg(test)]
mod tests;
