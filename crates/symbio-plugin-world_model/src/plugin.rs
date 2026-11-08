//! world_model 插件本体 —— **世界状态观察 + 读数**（名册 #35）。
//!
//! 本插件只做三件事：给模型一个能记下世界状态观察的工具（`world_observe`）、把每次
//! 成功观察落成一条领域事实（`world.observed`，经行为端口交草稿）、把那些事实折成一张
//! 可读的世界模型视图（`world.model`，对外可读）。
//!
//! ## 为什么这三件事必须同包
//!
//! 没有写入方，视图永远是空的；没有视图，事实只能靠读转写看。分成两个包就得让其中
//! 一个跨包读别人的私有事实——那正是 D-221 要消灭的形状。故同包落地。
//!
//! ## 本插件不认识任何兄弟插件
//!
//! 它不 import、不持有 `loop` / `session` / 别的能力包：写入走**行为端口**（宿主按
//! 名字选实现，`loop` 不认识 `world_observe`），读出走**视图名**（读方按名字读，
//! 不依赖本 crate）。卸载它 ⇒ 工具面少一个、视图格消失、事实不再产生，而系统照常
//! 运行（T3 形状）。
//!
//! ## 无自有协议、无配置
//!
//! 观察的入口就是工具本身（模型侧），读出口是视图（宿主侧）。造一条 `world_model/*`
//! 路由只会多一份校验、一份广播、一份前端渲染，而没有任何一个真实调用方需要它。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_WORLD_MODEL;
use symbio_plugin_api::{
    Capability, CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::WorldObserveTool;

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "世界模型";

/// 世界模型插件（无状态：它不持有任何事实源，写入全走行为端口）
pub struct WorldModelPlugin {
    tools: Arc<Vec<Arc<dyn Capability>>>,
}

impl WorldModelPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(WorldModelPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![Arc::new(WorldObserveTool)]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_WORLD_MODEL, PLUGIN_TITLE)
            .with_description("世界模型：把世界状态观察记成领域事实，并折成可读的当前世界视图。")
            .with_version("0.1.0")
    }
}

impl Default for WorldModelPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for WorldModelPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 观察的入口是工具（模型侧），读出口是视图（`world.model`，按名字读）。
    /// 两条都比一条自造协议更短，且都不需要本插件认识调用方。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "world_model 无自有协议路由 `{path}`：观察走工具 `world_observe`，读出口是视图 `world.model`"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(format!("未知遍历路径: {sub_path}")));
        }

        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            for tool in self.tools.iter() {
                visitor.put(CapabilityRow::tool(tool.clone())).await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(
    PLUGIN_ID_WORLD_MODEL,
    WorldModelPlugin::build,
    dyn Plugin
);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
