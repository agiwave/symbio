//! forecast 插件本体：本能力没有工具与路由，扩展面走 `symbio-plugin-api` 的链接期
//! 收集，不需要插件实例参与。
//!
//! 实例仍然要有：装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 视图随 crate
//! 一起从进程里消失。这里是那个挂载点本身。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_FORECAST;
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH,
};

/// 插件的展示标题（元数据与视图面共用一处）。
pub const PLUGIN_TITLE: &str = "未来预测";

pub struct ForecastPlugin;

impl ForecastPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ForecastPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_FORECAST, PLUGIN_TITLE)
            .with_description("未来预测：把任务事实折成「还剩多少没完成」的读数视图。")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for ForecastPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面，也没有公开视图：`forecast.remaining` 只在本包内折，
    /// 今天没有第二个读者。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[forecast] 本插件没有路由：{path}"
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

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_FORECAST, ForecastPlugin::build, dyn Plugin);
