//! 插件本体：本能力没有工具与路由，编码走收束贡献（链接期收集），
//! 不需要插件实例参与判决。
//!
//! 实例仍然要有：装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 它的声明与
//! 贡献随 crate 一起从进程里消失——情景记忆**不再编码**（巩固与召回仍在 loop，
//! 它们读的是已落格的事实）。这里是那个挂载点本身。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_MEMORY_EPISODIC;
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH,
};

pub struct MemoryEpisodicPlugin;

impl MemoryEpisodicPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(MemoryEpisodicPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_MEMORY_EPISODIC, "情景记忆")
            .with_description("轮末两条编码：用户发言（经验）与成功收束正文（判断）")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for MemoryEpisodicPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：声明与贡献都走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[memory_episodic] 本插件没有路由：{path}"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(
    PLUGIN_ID_MEMORY_EPISODIC,
    MemoryEpisodicPlugin::build,
    dyn Plugin
);
