//! Agent 统一资源协议（resources/*）：以 zip 上传为主，文件名即 Agent 目录名。

use crate::plugins::agent::manager::AgentRegistry;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::resources::{
    capabilities_for, decode_zip_b64, extract_zip_to_entity, ResourceDeleteRequest,
    ResourceSummary, ResourceUploadRequest, ResourceUploadResponse, ResourcesListResponse,
    RESOURCE_AGENT,
};
use crate::symbio_core::{
    create_object, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};
use std::sync::Arc;

fn es(ctx: &Arc<dyn InvokeRequest>) -> Result<std::sync::Arc<dyn crate::symbio_core::providers::StorageService>, PluginError> {
    create_object::<dyn crate::symbio_core::providers::StorageService>("storage_service", ctx.clone())
        .ok_or_else(|| PluginError::InternalError("storage_service 不可用".to_string()))
}

/// resources/list — 列出全部 Agent（统一 ResourceSummary 契约）
pub async fn handle_list(
    plugin: &AgentPlugin,
    _ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    {
        let agent_config = plugin.config.read().await;
        let _ = AgentRegistry::ensure_initialized(&plugin.manager, workdir_opt, &agent_config).await;
    }
    let agents = plugin.manager.list_agents(workdir_opt).await;

    let items = agents
        .into_iter()
        .map(|a| {
            let mut it = ResourceSummary::new(RESOURCE_AGENT, &a.id, a.name.clone());
            it.description = Some(a.description.clone());
            it.status = "active".to_string();
            it
        })
        .collect::<Vec<_>>();

    let resp = ResourcesListResponse {
        kind: RESOURCE_AGENT.to_string(),
        capabilities: capabilities_for(RESOURCE_AGENT),
        items,
    };
    Ok(PluginPayload::new(&resp))
}

/// resources/upload — 上传 zip 创建/更新 Agent（zip 根含 profile.json / identity 等）
pub async fn handle_upload(
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceUploadRequest = ctx.payload()?;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PluginError::ValidationError("Agent 资源名称不能为空".to_string()))?
        .to_string();
    let b64 = req.zip_b64.as_deref().ok_or_else(|| {
        PluginError::ValidationError("Agent 以 zip 上传（zip_b64）".to_string())
    })?;
    let bytes = decode_zip_b64(b64)?;

    let store = es(&ctx)?;
    let entity_store = store.entity_store();
    let category = crate::symbio_core::providers::categories::AGENT;
    extract_zip_to_entity(entity_store, category, &name, &bytes).await?;

    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: RESOURCE_AGENT.to_string(),
        id: name,
        created: true,
    }))
}

/// resources/delete — 删除 Agent（物理目录 + 缓存清理）
pub async fn handle_delete(
    _plugin: &AgentPlugin,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceDeleteRequest = ctx.payload()?;
    let store = es(&ctx)?;
    let entity_store = store.entity_store();
    let category = crate::symbio_core::providers::categories::AGENT;

    match entity_store.delete_entity(category, &req.id).await {
        Ok(()) => {}
        Err(crate::symbio_core::providers::EntityStoreError::NotFound { .. }) => {
            crate::plugin_warn!("agent", "磁盘上已无 agent {} 目录", req.id);
        }
        Err(e) => {
            return Err(PluginError::InternalError(format!("删除 Agent 失败: {e}")));
        }
    }

    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: RESOURCE_AGENT.to_string(),
        id: req.id,
        created: false,
    }))
}