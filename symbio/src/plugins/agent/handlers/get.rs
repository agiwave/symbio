use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};
use std::sync::Arc;

/// `agent/get` 处理器：按 ID 获取 Agent profile
///
/// 解析顺序：
/// 1. 优先从 ctx 的 `AGENT_ID` SymbioKey 取值
/// 2. 否则尝试从 payload.id 提取
///
/// 错误处理：
/// - 缺少 id => `PluginError::ValidationError`
/// - 找不到 Agent => `PluginError::NotFound`
pub async fn handle(
    plugin: &AgentPlugin,
    ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    let id = ctx
        .get(crate::symbio_core::AGENT_ID)
        .or_else(|| {
            ctx.payload::<serde_json::Value>().ok().and_then(|v| {
                v.get(cu_fields::ID)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        })
        .ok_or_else(|| PluginError::ValidationError("Missing id".to_string()))?;

    match plugin.manager.get_agent(workdir_opt, &id).await {
        Some(profile) => Ok(PluginPayload::new(&profile)),
        None => Err(PluginError::NotFound(format!("Agent '{id}' not found"))),
    }
}

#[cfg(test)]
#[path = "get_tests.rs"]
mod tests;
