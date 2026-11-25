//! MCP 工具的端口执行体包装
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
//! - 销毁：随 `tool_visitor`（通常是 `collectors::DefaultToolVisitor`）一起被丢弃
//! - **不持有**任何 stdio 进程 / http 连接（按需 lazy 加载）

use crate::schemas::mcp_config::McpServerConfig;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use symbio_plugin_api::collectors::FactsOf;
use symbio_plugin_api::{
    missing_caller, port_result, CapabilityCategory, CapabilityMeta, ExecEnv, PluginError,
    PluginInvokeRequest, PortExecContext, PortExecutor, PortInput, PortOutcome, SandboxFact,
};

use super::manager::McpManager;
use super::types::{McpTool, McpToolCallResponse};

/// 单个 MCP 工具的端口执行体包装
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

    pub fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: Self::namespaced_name(&self.server_name, &self.tool_name),
            description: format!("[MCP:{}] {}", self.server_name, self.description),
            input_schema: self.input_schema.clone(),
            category: Some(CapabilityCategory::Mcp),
            examples: None,
            ..Default::default()
        }
    }

    /// 注册给审批闸门的事实提取器：工具身份固定，参数语义按本次调用提取。
    pub(crate) fn facts(&self) -> FactsOf {
        let tool_name = Self::namespaced_name(&self.server_name, &self.tool_name);
        Arc::new(move |args, _ctx| intent_facts(&tool_name, args))
    }
}

#[derive(Clone, Copy)]
enum ArgumentKind {
    Path,
    Url,
    Command,
}

/// MCP 工具调用的沙箱事实。所有 server / tool 共用这套投影，不依赖特定 MCP 实现。
pub(crate) fn intent_facts(tool_name: &str, args: &Value) -> Vec<SandboxFact> {
    let mut facts = vec![SandboxFact::new("mcp_tool", tool_name)];
    collect_argument_facts(args, None, &mut facts);
    facts
}

fn collect_argument_facts(
    value: &Value,
    inherited_kind: Option<ArgumentKind>,
    facts: &mut Vec<SandboxFact>,
) {
    match value {
        Value::String(value) => {
            let value = redact_embedded_credentials(value);
            facts.push(SandboxFact::new("mcp_argument", value.clone()));
            match inherited_kind {
                Some(ArgumentKind::Path) => facts.push(SandboxFact::new(
                    "vdfs_path",
                    symbio_plugin_api::vdfs_normalize_path(&value),
                )),
                Some(ArgumentKind::Url) => {
                    let normalized = url::Url::parse(&value)
                        .map(|mut url| {
                            url.set_fragment(None);
                            url.to_string()
                        })
                        .unwrap_or_else(|_| value.trim().to_string());
                    facts.push(SandboxFact::new("url", normalized));
                }
                Some(ArgumentKind::Command) => {
                    facts.push(SandboxFact::new("shell_command", value.trim()));
                }
                None => {}
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_argument_facts(value, inherited_kind, facts);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                // 凭据字段及其整棵子树都不进入规则 / 审计可见的事实。
                if is_sensitive_key(key) {
                    continue;
                }
                collect_argument_facts(value, argument_kind(key), facts);
            }
        }
        _ => {}
    }
}

fn redact_embedded_credentials(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return value.to_string();
    };
    if url.username().is_empty() && url.password().is_none() {
        return value.to_string();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "credential",
        "authorization",
        "cookie",
        "api_key",
        "apikey",
        "key",
    ]
    .iter()
    .any(|marker| key.contains(marker))
}

fn argument_kind(key: &str) -> Option<ArgumentKind> {
    let key = key.to_ascii_lowercase();
    if ["path", "file", "dir", "directory", "workdir"]
        .iter()
        .any(|marker| key.contains(marker))
    {
        Some(ArgumentKind::Path)
    } else if ["url", "uri", "endpoint"]
        .iter()
        .any(|marker| key.contains(marker))
    {
        Some(ArgumentKind::Url)
    } else if ["command", "shell", "exec"]
        .iter()
        .any(|marker| key.contains(marker))
    {
        Some(ArgumentKind::Command)
    } else {
        None
    }
}

#[async_trait]
impl PortExecutor for McpToolCapability {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl McpToolCapability {
    async fn run(
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
