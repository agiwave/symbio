//! Task 插件本体：本能力没有工具与路由，四条扩展面都走 `symbio-plugin-api` 的
//! 链接期收集，不需要插件实例参与。
//!
//! 实例仍然要有：装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒ 它的行为 /
//! 贡献 / 段 / 声明随 crate 一起从进程里消失。这里是那个挂载点本身。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_core::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PATH,
};
use symbio_deploy::PLUGIN_ID_TASK;

pub struct TaskPlugin;

impl TaskPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(TaskPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_TASK, "任务")
            .with_description("任务表声明、独立验证与返工、就绪集调度")
            .with_version("0.1.14")
    }
}

#[async_trait]
impl Plugin for TaskPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：行为 / 声明 / 段 / 贡献都走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[task] 本插件没有路由：{path}"
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

symbio_core::submit_object_creator!(PLUGIN_ID_TASK, TaskPlugin::build, dyn Plugin);
