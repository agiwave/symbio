use crate::plugins::agent::manager::AgentRegistry;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::{InvokeRequest, InvokeResponse, PluginPayload};
use std::sync::Arc;

pub async fn handle(
    plugin: &AgentPlugin,
    _ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    crate::plugin_info!("agent", "Listing agents, workdir: {:?}", workdir_opt);

    {
        let agent_config = plugin.config.read().await;
        match AgentRegistry::ensure_initialized(&plugin.manager, workdir_opt, &agent_config).await {
            Ok(_) => crate::plugin_info!("agent", "Agent registry initialized successfully"),
            Err(e) => crate::plugin_error!("agent", "Failed to initialize agent registry: {}", e),
        }
    }

    let agents = plugin.manager.list_agents(workdir_opt).await;
    crate::plugin_info!("agent", "Found {} agents", agents.len());

    if agents.is_empty() {
        crate::plugin_warn!("agent", "No agents found");
    } else {
        for agent in &agents {
            crate::plugin_info!(
                "agent",
                "Agent: id={}, name={}, base_dir={:?}",
                agent.id,
                agent.name,
                agent.base_dir
            );
        }
    }

    Ok(PluginPayload::new(&agents))
}
