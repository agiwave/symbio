use crate::plugins::agent::core::AgentConfig;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, InvokeResponse, PluginPayload};
use std::sync::Arc;

pub async fn handle_get(
    plugin: &AgentPlugin,
    _ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let cfg = plugin.config.read().await;
    Ok(PluginPayload::new(&*cfg))
}

pub async fn handle_set(
    plugin: &AgentPlugin,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let new_cfg: AgentConfig = ctx.payload()?;
    {
        let mut cfg = plugin.config.write().await;
        *cfg = new_cfg;
    }
    if let Some(p) = plugin.get_parent().await {
        let save_ctx = ctx.fork();
        save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
        // save_config 失败不再静默吞错，
        // 但仍向调用方返回 success（避免前端误判）；写 warn 便于排障
        if let Err(e) = p.route(save_ctx).await {
            crate::plugin_warn!(
                "agent",
                "handle_set: save_config persistence failed err={:?}",
                e
            );
        }
    }
    Ok(PluginPayload::new(&SimpleResponse::success()))
}
