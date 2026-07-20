use crate::plugins::agent::core::types::cu_fields;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};
use serde_json::json;
use std::sync::Arc;

/// `agent/delete` 处理器：删除一个 Agent 的物理目录
///
/// 输入：
/// - `id`：智能体 ID（必需，从 ctx.AGENT_ID 或 payload.id 解析）
/// - `workdir`：通过 ctx.WORKDIR 注入（handler 已从 plugin 透传）
///
/// 返回：
/// - 成功：`PluginPayload::new(&{"deleted": true})`
/// - agent 不存在：`PluginPayload::new(&{"deleted": false})`（幂等，不报错）
/// - 缺 id：`PluginError::ValidationError`
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

    match plugin.manager.delete_agent(workdir_opt, &id).await {
        Ok(true) => Ok(PluginPayload::new(&json!({ "deleted": true, "id": id }))),
        Ok(false) => Ok(PluginPayload::new(&json!({ "deleted": false, "id": id }))),
        Err(e) => Err(PluginError::InternalError(e)),
    }
}

#[cfg(test)]
#[path = "delete_tests.rs"]
mod tests;
