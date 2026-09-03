// 所有子模块私有——通过 route() 统一入口对外暴露
mod chat;
mod config;
mod create;
mod resources;
pub(crate) mod system_prompt;

use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::resources::{
    RESOURCES_DELETE, RESOURCES_LIST, RESOURCES_UPLOAD,
};
use crate::symbio_core::{
    InvokeRequest, InvokeResponse, PluginError, PluginPayload, CONFIG_GET, CONFIG_SET,
};
use std::sync::Arc;

/// Agent plugin 的统一路由入口
///
/// `plugin.rs` 只需调用 `handlers::route(...)`，不直接依赖各 handler 实现模块。
/// 新增 handler 只需在此处 match 加一行——plugin 完全不需要改动 import。
///
/// 接收 `Arc<AgentPlugin>` 而非 `&AgentPlugin`：让 handler 内部能 new 需要
/// `Arc<AgentPlugin>` 的工具（如 `AgentCreateTool::new(plugin)`）。
pub async fn route(
    plugin: Arc<AgentPlugin>,
    path: &str,
    ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    match path {
        "chat" => chat::handle(&plugin, ctx, workdir_opt).await,
        "create" => create::handle(plugin, ctx, workdir_opt).await,
        // 统一资源协议
        RESOURCES_LIST => resources::handle_list(&plugin, ctx, workdir_opt).await,
        RESOURCES_UPLOAD => resources::handle_upload(ctx).await,
        RESOURCES_DELETE => resources::handle_delete(&plugin, ctx).await,
        CONFIG_GET => config::handle_get(&plugin, ctx).await,
        CONFIG_SET => config::handle_set(&plugin, ctx).await,
        // 内部生命周期路由：用于宿主在进程退出前显式触发清理
        "internal/shutdown_all" => {
            plugin.manager.shutdown_all().await;
            Ok(PluginPayload::Empty)
        }
        _ => Err(PluginError::NotFound(format!("Unknown path: {path}"))),
    }
}
