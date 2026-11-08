//! 幂等台账插件本体：只做「注册一个工具 + 不提供路由」。
//!
//! 事实 / 行为 / 视图走 `symbio-plugin-api` 的链接期收集，不需要插件实例参与。
//! **工具是实例提供的**：停用插件 ⇒ 实例不构造 ⇒ `traverse` 不注册 ⇒ 工具从模型
//! 工具面消失（与 T88 证明的同一条装配位语义）。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_IDEMPOTENCY;
use symbio_plugin_api::{
    CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
    TRAVERSE_AVAILABLE_TOOLS,
};

use super::tool::IdempotentRecordTool;

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "幂等台账";

pub struct IdempotencyPlugin {
    tools: Arc<Vec<Arc<dyn symbio_plugin_api::Capability>>>,
}

impl IdempotencyPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(IdempotencyPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![Arc::new(IdempotentRecordTool)]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_IDEMPOTENCY, PLUGIN_TITLE)
            .with_description("幂等台账：同一个键在同一会话里只记一次，重复的调用在执行前被拒。")
            .with_version("0.1.0")
    }
}

impl Default for IdempotencyPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for IdempotencyPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 记录的入口是工具（模型侧），折叠留在本包的视图 `idempotency.ledger` 里。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "idempotency 无自有协议路由 `{path}`：记录走工具 `idempotent_record`，折叠在本包视图 `idempotency.ledger`（不对外）"
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
    PLUGIN_ID_IDEMPOTENCY,
    IdempotencyPlugin::build,
    dyn Plugin
);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
