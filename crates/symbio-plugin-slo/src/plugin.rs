//! 本插件本体：**挂载点本身**，没有工具、没有路由、没有行为。
//!
//! 三条视图都走 `symbio-plugin-api` 的链接期收集，不需要实例参与；实例仍然要有——
//! 装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 这三列读数从插件树里消失。这里是
//! 那个挂载点本身（与 `symbio-plugin-task` 同一形状、同一理由）。
//!
//! 给它一个 `route` 臂只会多一条永远 `NotFound` 的地址，给它一个 `traverse` 臂只会多一个
//! 永远空的工具面——两样都是「注册了但没人能到达」的形状（`E-012` 判过同一个病）。
//! 所以 `route` 恒 `NotFound`、`traverse` 恒空数组，两者都不是占位符而是**答案**。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_SLO;
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH,
};

/// 插件的展示标题（元数据一处定义）。
pub const PLUGIN_TITLE: &str = "时延分层";

pub struct SloPlugin;

impl SloPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(SloPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_SLO, PLUGIN_TITLE)
            .with_description("时延 / 兜底 / 跨度三列：按派发档折叠，单片与跨会话两半")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for SloPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：三条视图都走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[slo] 本插件没有路由：{path}"
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

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_SLO, SloPlugin::build, dyn Plugin);
