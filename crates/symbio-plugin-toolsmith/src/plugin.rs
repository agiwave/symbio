//! toolsmith 插件本体 —— **制造工具（L2 受限）**（名册 #41）。
//!
//! 本插件只做三件事：给模型一个能登记自身工具定义的工具（`toolsmith_define`）、把每次
//! 成功登记落成一条领域事实（`toolsmith.defined`，经行为端口交草稿）、把那些事实折成
//! 一张当前定义清单（`toolsmith.recipes`，对外可读）。
//!
//! ## 受限边界（写在这里，不由调用方猜）
//!
//! 定义只进清单，**不改本轮工具面**：运行期装配不存在（D-217），造出来的工具进入可调用
//! 面要等下一次装配——那是名册 #42（L3）的事。本包的 owner 常量与视图过滤保证「只能造
//! 自己插件的工具」，将来装配期按 owner 读即可，不需要改本包形状。
//!
//! ## 本插件不认识任何兄弟插件
//!
//! 它不 import、不持有 `loop` / `session` / 别的能力包：写入走**行为端口**，读出走
//! **视图名**。卸载它 ⇒ 工具面少一个、视图格消失、定义不再产生，而系统照常运行。
//!
//! ## 无自有协议、无配置
//!
//! 登记的入口就是工具本身（模型侧），读出口是视图（宿主侧）。造一条 `toolsmith/*` 路由
//! 只会多一份校验、一份广播、一份前端渲染，而没有任何一个真实调用方需要它。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_TOOLSMITH;
use symbio_plugin_api::{
    Capability, CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::ToolsmithDefineTool;

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "制造工具";

/// 制造工具插件（无状态：它不持有任何事实源，写入全走行为端口）
pub struct ToolsmithPlugin {
    tools: Arc<Vec<Arc<dyn Capability>>>,
}

impl ToolsmithPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ToolsmithPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![Arc::new(ToolsmithDefineTool)]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_TOOLSMITH, PLUGIN_TITLE)
            .with_description("制造工具（L2 受限）：只能登记本插件自己的工具定义，并折成可读清单。")
            .with_version("0.1.0")
    }
}

impl Default for ToolsmithPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ToolsmithPlugin {
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
            "toolsmith 无自有协议路由 `{path}`：登记走工具 `toolsmith_define`，读出口是视图 `toolsmith.recipes`"
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

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_TOOLSMITH, ToolsmithPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
