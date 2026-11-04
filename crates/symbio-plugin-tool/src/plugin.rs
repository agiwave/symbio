//! 工具世界插件的门面：为 `port.tool` 注册提供者行，并申报工具债行。
//!
//! 本插件没有自己的路由命令（工具执行不经 `route`，只经端口提供者），所以 `route`
//! 诚实回落 NotFound；`traverse` 是唯一的装配点——把提供者行与债行交给收集宿主。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::{
    port_rows, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PortProvider, CAPABILITY_VISITOR, PATH,
    PLUGIN_ID_TOOL,
};

use super::port::ToolPort;

/// 工具世界插件（无状态：执行期从调用方 ctx 读能力表）。
pub(crate) struct ToolPlugin;

impl ToolPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ToolPlugin) as Arc<dyn Plugin>
    }

    fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_TOOL, "Tool")
            .with_description("工具世界：port.tool 的执行、路由与结果提取")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for ToolPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[tool] 未知子命令: {path}（工具执行走端口提供者，不经 route）"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            let provider: Arc<dyn PortProvider> = Arc::new(ToolPort);
            for row in port_rows(&provider) {
                visitor.put(row).await;
            }
            for row in super::debt::debt_rows() {
                visitor.put(row).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_TOOL, ToolPlugin::build, dyn Plugin);
