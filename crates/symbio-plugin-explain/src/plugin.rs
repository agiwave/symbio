//! explain 插件本体 —— **可解释**（名册 #49）。
//!
//! 本插件只做三件事：给模型一个能记下决定理由的工具（`explain_record`）、把每次成功
//! 记录落成一条领域事实（`explain.rationale`，经行为端口交草稿）、把那些事实折成一张
//! 可读的理由台账（`explain.rationales` 视图，名字只在本包内用）。
//!
//! ## 本插件不认识任何兄弟插件
//!
//! 它不 import、不持有 `loop` / `session` / 别的能力包：写入走**行为端口**（宿主按
//! 名字选实现，`loop` 不认识 `explain_record`），读出走**视图名**（读方按名字读，
//! 不依赖本 crate）。卸载它 ⇒ 工具面少一个、视图格消失、事实不再产生，而系统照常
//! 运行（T3 形状）。
//!
//! ## 无自有协议、无配置
//!
//! 理由记录的入口就是工具本身（模型侧），读出口是视图（宿主侧）。造一条 `explain/*`
//! 路由只会多一份校验、一份广播、一份前端渲染，而没有任何一个真实调用方需要它。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_EXPLAIN;
use symbio_plugin_api::{
    Capability, CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::ExplainTool;

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "可解释";

/// 可解释插件（无状态：它不持有任何事实源，写入全走行为端口）
pub struct ExplainPlugin {
    tools: Arc<Vec<Arc<dyn Capability>>>,
}

impl ExplainPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ExplainPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![Arc::new(ExplainTool)]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_EXPLAIN, PLUGIN_TITLE)
            .with_description("可解释：把一次决定的理由记成领域事实，并折成可读的理由台账视图。")
            .with_version("0.1.0")
    }
}

impl Default for ExplainPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ExplainPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 记录的入口是工具（模型侧），折叠留在本包的视图 `explain.rationales` 里。
    /// 两条都比一条自造协议更短，且都不需要本插件认识调用方。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "explain 无自有协议路由 `{path}`：记录走工具 `explain_record`，折叠在本包视图 `explain.rationales`（不对外）"
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

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_EXPLAIN, ExplainPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
