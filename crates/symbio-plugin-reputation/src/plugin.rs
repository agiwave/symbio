//! 本插件本体：**挂载点本身**，没有工具、没有路由、没有行为。
//!
//! 那条视图走 `symbio-plugin-api` 的链接期收集，不需要实例参与；实例仍然要有——装配清单
//! 按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 声誉台账这一列从插件树里消失。这里是那个
//! 挂载点本身（与 `symbio-plugin-cost` 同一形状、同一理由）。
//!
//! 给它一个 `route` 臂只会多一条永远 `NotFound` 的地址，给它一个 `traverse` 臂只会多一个
//! 永远空的工具面——两样都是「注册了但没人能到达」的形状（`E-012` 判过同一个病）。
//! 所以 `route` 恒 `NotFound`、`traverse` 恒空数组，两者都不是占位符而是**答案**。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_REPUTATION;
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH,
};

/// 插件的展示标题（元数据一处定义）。
pub const PLUGIN_TITLE: &str = "声誉台账";

pub struct ReputationPlugin;

impl ReputationPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ReputationPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_REPUTATION, PLUGIN_TITLE)
            .with_description("承诺三格按主体折算：淘汰回路与出口共用的那一个口径")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for ReputationPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：视图走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[reputation] 本插件没有路由：{path}"
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
    PLUGIN_ID_REPUTATION,
    ReputationPlugin::build,
    dyn Plugin
);
