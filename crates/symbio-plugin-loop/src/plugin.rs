//! 插件身份：名字、清单与注册入口保持不变，内容只剩纯机制环。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_deploy::PLUGIN_ID_LOOP;
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload,
};
use symbio_plugin_api::drive::drive_row;
use symbio_plugin_api::CAPABILITY_VISITOR;

/// 单环驱动器（crate 内唯一实例；对外经 creator 注册可见）。
pub(crate) struct LoopPlugin;

impl LoopPlugin {
    /// 静态工厂。
    pub(crate) fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(LoopPlugin)
    }

    /// 插件元数据。
    pub(crate) fn metadata() -> PluginMeta {
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
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            visitor
                .put(drive_row("loop", Arc::new(crate::drive::LoopDrive)))
                .await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_LOOP, LoopPlugin::build, dyn Plugin);
