//! 插件身份：名字、清单与注册入口保持不变，内容只剩纯机制环。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_deploy::PLUGIN_ID_LOOP;
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload,
};

/// 单环驱动器。
pub struct LoopPlugin;

impl LoopPlugin {
    /// 静态工厂。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(LoopPlugin)
    }

    /// 插件元数据。
    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_LOOP, "单环驱动器")
            .with_description("纯机制环：选债、派发、等待、配对、记账、预算与租约、重放。")
            .with_version("0.1.0")
            .with_order(90)
            .with_icon("loop")
            .with_hidden(true)
    }
}

#[async_trait]
impl Plugin for LoopPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_sdk::keys::PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!("未知路径: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_LOOP, LoopPlugin::build, dyn Plugin);
