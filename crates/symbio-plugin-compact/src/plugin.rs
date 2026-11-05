//! 挂载点：端口提供者行与债行都由本 crate 在自己的 `traverse` 里申报。
//!
//! 没有路由、没有工具、没有行为：扩展面走能力表，实例只为**挂载**——装配清单按
//! 插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 两行都不出现（环在调用点读不到 `compact`
//! 端口的提供者，也就不欠压缩）。

use std::sync::Arc;

use async_trait::async_trait;
use symbio_plugin_api::{
    port_rows, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PortProvider, CAPABILITY_VISITOR, PATH,
    PLUGIN_ID_COMPACT,
};

use crate::debt::compact_debt_rows;
use crate::port::CompactPort;

pub(crate) struct CompactPlugin;

impl CompactPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。
    pub(crate) fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(CompactPlugin) as Arc<dyn Plugin>
    }

    fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_COMPACT, "上下文压缩")
            .with_description("超水位时把上下文压成快照：落 session.consolidated，读侧零改动")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for CompactPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[compact] 本插件没有路由：{path}"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            let provider: Arc<dyn PortProvider> = Arc::new(CompactPort);
            for row in port_rows(&provider) {
                visitor.put(row).await;
            }
            for row in compact_debt_rows() {
                visitor.put(row).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_COMPACT, CompactPlugin::build, dyn Plugin);
