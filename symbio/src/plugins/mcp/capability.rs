//! MCP 工具的 `Capability` 包装
//!
//! 每个远程 MCP 工具对应一个 `McpToolCapability` 实例；实例由
//! `McpPlugin::traverse` 在每次 `parent.traverse` 时根据 `manager.discover_tools`
//! 的返回值动态构造并注册到 `tool_visitor`。
//!
//! ## 命名
//!
//! 工具名采用 `mcp.<server_name>.<tool_name>` 三段式，避免不同 server
//! 之间的同名工具冲突（例如两个 server 都暴露 `search` 工具）。
//!
//! ## 生命周期
//!
//! - 创建：每次 `traverse` 时构造
//! - 销毁：随 `tool_visitor`（通常是 `providers::DefaultToolVisitor`）一起被丢弃
//! - **不持有**任何 stdio 进程 / http 连接（按需 lazy 加载）

use crate::plugins::mcp::schemas::mcp_config::McpServerConfig;
use crate::symbio_core::{
    Capability, CapabilityCategory, CapabilityMeta, ExecEnv, PluginError, PluginInvokeRequest,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use super::manager::McpManager;
use super::types::{McpTool, McpToolCallResponse};

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

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
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

        // 把 MCP 的**内容块形状**压平成本插件内的一步，再以工具结果的通用形状
        // 交出（`{"content": <文本>}`）。
        //
        // 为什么压平放在这里而不是会话层：`content` 是**内容块数组**（`text` /
        // `image` / `resource`），那是 MCP 自己的协议形状；会话层只认工具结果的
        // 通用字段名（`content` 为字符串）。跨插件约定越窄越好——把 MCP 的形状
        // 外泄出去，会话层就得为每个外部协议长一个分支。
        let call_result: McpToolCallResponse =
            serde_json::from_value(result.clone()).unwrap_or_default();
        let text = call_result.text();
        let mut out = serde_json::json!({ "content": text });
        // 结构化结果（规范可选）一并带上：会话层按 `content` 取正文，
        // 其余字段留在存储里供排查。
        if let Some(structured) = call_result.structured_content {
            out["structured_content"] = structured;
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "capability.test.rs"]
mod tests;
