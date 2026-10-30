//! Notes 插件本体：只做「注册一个工具 + 不提供路由」。
//!
//! 其余三条扩展面（声明 / 上下文 / 行为）走 `symbio-plugin-api` 的链接期收集，
//! 不需要插件实例参与。**工具是实例提供的**：停用插件 ⇒ 实例不构造 ⇒
//! `traverse` 不注册 ⇒ 工具从模型工具面消失（与 T88 证明的同一条装配位语义）。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_NOTES;
use symbio_plugin_api::{
    CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::NotesTool;

pub struct NotesPlugin;

impl NotesPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(NotesPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_NOTES, "便签")
            .with_description("记一条领域事实（Domain×Opened），并把最近的便签作为上下文段喂回")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for NotesPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：工具走能力收集，事实走宿主写口。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[notes] 本插件没有路由：{path}"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        if path == TRAVERSE_AVAILABLE_TOOLS {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                visitor.put(CapabilityRow::tool(Arc::new(NotesTool))).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_NOTES, NotesPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
