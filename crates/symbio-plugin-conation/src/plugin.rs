//! 本插件本体：**挂载点本身**，没有工具、没有路由、没有行为。
//!
//! 声明面（事实 / 视图 / 归宿行 / 多片合并）都走 `symbio-plugin-api` 的链接期收集，
//! 不需要实例参与；实例仍然要有——装配清单按插件目录挂载，停用插件 ⇒ 实例不构造 ⇒
//! 这个能力包从插件树里消失。债行与端口提供者行同一条装配位：停用后环既读不到
//! `conation.promote` 这笔债，也解析不到 `conation` 端口的执行体。
//!
//! 给它一个 `route` 臂只会多一条永远 `NotFound` 的地址，所以 `route` 恒 `NotFound`——
//! 那不是占位符而是**答案**（`E-012` 判过同一个病）。

use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::port_rows;
use symbio_plugin_api::PortProvider;
use symbio_plugin_api::PLUGIN_ID_CONATION;
use symbio_plugin_api::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH,
};

use crate::port::ConationPort;

/// 插件的展示标题（元数据一处定义）。
pub const PLUGIN_TITLE: &str = "目标自生成";

pub struct ConationPlugin;

impl ConationPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）。无状态、无配置。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(ConationPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_CONATION, PLUGIN_TITLE)
            .with_description("「欲」的表达与它的升格读数：一条事实、两条视图、一笔升格债")
            .with_version("0.1.16")
    }
}

#[async_trait]
impl Plugin for ConationPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件不占用路由面：事实 / 视图 / 归宿行都走链接期收集。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[conation] 本插件没有路由：{path}"
        )))
    }

    /// 装配位的两行：升格债（计划层读）与端口提供者（派发层解析）。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            visitor.put(crate::debt::row()).await;
            let provider: Arc<dyn PortProvider> = Arc::new(ConationPort);
            for row in port_rows(&provider) {
                visitor.put(row).await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_CONATION, ConationPlugin::build, dyn Plugin);
