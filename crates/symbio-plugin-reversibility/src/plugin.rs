//! reversibility 插件本体 —— **动作可逆等级 + 读数**（名册 #40）。
//!
//! 本插件只做三件事：给模型一个能标注动作可逆等级的工具（`reversibility_annotate`）、
//! 把每次成功标注落成一条领域事实（`reversibility.assessed`，经行为端口交草稿）、把那些
//! 事实折成一张可读的登记表（`reversibility.register`，名字只在本包内用）。
//!
//! ## 为什么这三件事必须同包
//!
//! 没有写入方，登记表永远是空的；没有登记表，判断只能靠读转写看。分成两个包就得让其中
//! 一个跨包读别人的私有事实——那正是 D-221 要消灭的形状。故同包落地。
//!
//! ## 本插件不认识任何兄弟插件
//!
//! 它不 import、不持有 `loop` / `session` / 别的能力包：写入走**行为端口**（宿主按名字选
//! 实现，`loop` 不认识 `reversibility_annotate`），读出走**视图名**。卸载它 ⇒ 工具面少
//! 一个、视图格消失、事实不再产生，而系统照常运行（T3 形状）。
//!
//! ## 无自有协议、无配置
//!
//! 标注的入口就是工具本身（模型侧），读出口是视图（宿主侧）。造一条
//! `reversibility/*` 路由只会多一份校验、一份广播、一份前端渲染，而没有任何一个真实调
//! 用方需要它。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_REVERSIBILITY;
use symbio_plugin_api::{
    Capability, CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::ReversibilityAnnotateTool;

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "可逆性设计";

/// 可逆性插件（无状态：它不持有任何事实源，写入全走行为端口）
pub struct ReversibilityPlugin {
    tools: Arc<Vec<Arc<dyn Capability>>>,
}

impl ReversibilityPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ReversibilityPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![Arc::new(ReversibilityAnnotateTool)]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_REVERSIBILITY, PLUGIN_TITLE)
            .with_description("可逆性设计：给动作标注可逆等级与补偿做法，并折成可读的登记表。")
            .with_version("0.1.0")
    }
}

impl Default for ReversibilityPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ReversibilityPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 路由入口：**无自有协议**。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "reversibility 无自有协议路由 `{path}`：标注走工具 `reversibility_annotate`，折叠在本包视图 `reversibility.register`（不对外）"
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
    PLUGIN_ID_REVERSIBILITY,
    ReversibilityPlugin::build,
    dyn Plugin
);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
