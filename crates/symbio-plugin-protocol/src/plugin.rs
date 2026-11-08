//! protocol 插件本体 —— **抽象协议设计**（名册 #43）。
//!
//! 本插件做四件事：让模型提出协议（`protocol_propose`）、让参与者加入（`protocol_join`）、
//! 把每次成功调用落成领域事实（经行为端口交草稿）、把那些事实折成一张清单
//! （`protocol.catalog`，对外可读），并通过 `protocol/catalog` 路由把同一份折叠交给
//! **第二方**。
//!
//! ## 为什么这里需要一条路由
//!
//! 前三条能力（#35 / #40 / #41）的读方都住在宿主里；协议不一样：它的第二个读方是
//! **另一个实例 / 另一个代理**，不该被迫链接本 crate 才能问「现在有哪些协议、成立没有」。
//! 一条只读路由是最小的跨实例读入口，且不需要给对方任何类型。
//!
//! ## 路由为什么只读
//!
//! 写口只有一个（I-1，D-213）：路由拿事实源读现状，不拿它写。加入仍走工具与行为端口，
//! 于是「谁写的」在转写里有 `produced_by`，不需要再给路由开一条旁路。路由与视图调用
//! 同一个 [`super::decl::catalog`]，两边看到的成立状态不会分叉。
//!
//! ## 事实源的取法
//!
//! 优先用上下文里装配层下发的同一个句柄（`FACT_SOURCE`，I-1）；网关的 `/invoke` 只带
//! metadata，不含该句柄，于是退到 `SESSION_ID` + 已登记的开面（`fact_source_opener`）
//! 打开同一会话的事实源。两条路都只读，宿主与网关看到的是同一份事实。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::fact_source::{declared_fact_source_opener, FactSource};
use symbio_plugin_api::PLUGIN_ID_PROTOCOL;
use symbio_plugin_api::{
    Capability, CapabilityRow, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, CAPABILITY_VISITOR, FACT_SOURCE, PATH,
    SESSION_ID, TRAVERSE_AVAILABLE_TOOLS,
};

use super::decl::catalog;
use super::tool::{ProtocolJoinTool, ProtocolProposeTool};

/// 插件的展示标题（元数据与工具面共用一处）。
pub const PLUGIN_TITLE: &str = "抽象协议设计";

/// 协议插件（无状态：它不持有任何事实源，写入全走行为端口，读出按会话打开）
pub struct ProtocolPlugin {
    tools: Arc<Vec<Arc<dyn Capability>>>,
}

impl ProtocolPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ProtocolPlugin::new()) as Arc<dyn Plugin>
    }

    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![
                Arc::new(ProtocolProposeTool),
                Arc::new(ProtocolJoinTool),
            ]),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_PROTOCOL, PLUGIN_TITLE)
            .with_description(
                "抽象协议设计：提出协议、第二方加入后成立，并提供只读的 catalog 读入口。",
            )
            .with_version("0.1.0")
    }

    /// 取本会话的事实源：优先装配层下发的同一句柄，否则按 `session_id` 打开。
    fn fact_source(ctx: &Arc<dyn PluginInvokeRequest>) -> Result<Arc<dyn FactSource>, PluginError> {
        if let Some(source) = ctx.get(FACT_SOURCE) {
            return Ok(source);
        }
        let session_id: String = ctx.get(SESSION_ID).ok_or_else(|| {
            PluginError::ValidationError(
                "protocol/catalog 需要 session_id（或由装配层下发事实源）".to_string(),
            )
        })?;
        let opener = declared_fact_source_opener()
            .ok_or_else(|| PluginError::InternalError("构建里没有登记事实源开面".to_string()))?;
        opener
            .open(&session_id)
            .map_err(|e| PluginError::InternalError(format!("打开会话事实源失败：{e}")))
    }
}

impl Default for ProtocolPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for ProtocolPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 路由入口：`protocol/catalog`（**只读**）。
    ///
    /// 返回与视图 `protocol.catalog` 逐字一致的折叠值：第二方看到的成立状态，必须与
    /// 宿主里任何一个插件读到的相同。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            // 协议清单：第二方的只读读入口，与视图 `protocol.catalog` 同一折叠。
            "catalog" => {
                let source = Self::fact_source(&ctx)?;
                let value = catalog(&source.facts(), i64::MAX);
                Ok(PluginPayload::new(&value))
            }
            _ => Err(PluginError::NotFound(format!(
                "protocol 没有路由 `{path}`：读入口是 `protocol/catalog`，提出与加入走工具"
            ))),
        }
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

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_PROTOCOL, ProtocolPlugin::build, dyn Plugin);

/// 视图名与路由值共用一处：`routes.rs` 的生成常量必须与这里一致，否则第二方按
/// 公开地址调不到本插件的折叠。
pub(crate) const _ROUTE: &str = symbio_plugin_api::routes::ROUTE_PROTOCOL_CATALOG;

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
