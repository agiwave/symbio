//! Hook 插件 - 提供 Hook 事件机制
//!
//! Hook 通过插件路由机制实现：
//! - 其他插件通过 `hooks/fire` 路由触发 Hook
//! - Hook 执行结果通过 PluginMessage 返回
//! - 不需要在 symbio_core 中添加特殊处理

use super::executor::HookExecutor;
use super::registry::{HookRegistration, HookRegistry};
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::schemas::system::hook::{HookEvent, HookOutput};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, PLUGIN_HOOK,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct HooksPlugin {
    registry: Arc<RwLock<HookRegistry>>,
    executor: Arc<HookExecutor>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookFireRequest {
    pub session_id: String,
    pub event: HookEvent,
}

impl HooksPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let parent = ctx.parent();
        Arc::new(HooksPlugin::new(parent)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self {
            registry: Arc::new(RwLock::new(HookRegistry::new())),
            executor: Arc::new(HookExecutor::new()),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("hooks", "钩子插件")
            .with_description("提供事件钩子机制，支持插件间事件订阅与触发")
            .with_version("0.1.0")
    }

    async fn handle_fire(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<HookOutput> {
        let req: HookFireRequest = ctx.payload()?;
        let workdir = ctx.get(crate::symbio_core::WORKDIR).unwrap_or_default();

        let configs = self
            .registry
            .read()
            .await
            .get_hooks(req.event.event_name())
            .await;
        let result = self
            .executor
            .execute(configs.as_slice(), &req.event, &req.session_id, &workdir)
            .await;

        Ok(result)
    }

    async fn handle_register(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let reg: HookRegistration = ctx.payload()?;

        self.registry.write().await.register(&reg).await;

        Ok(PluginPayload::new(&SimpleResponse::ok()))
    }

    async fn handle_list(&self) -> InvokeResponse<PluginPayload> {
        let hooks = self.registry.read().await.list_hooks().await;
        Ok(PluginPayload::new(&hooks))
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }
}

#[async_trait]
impl Plugin for HooksPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        match path {
            "fire" => {
                let resp = self.handle_fire(ctx).await?;
                Ok(PluginPayload::new(&resp))
            }
            "register" => self.handle_register(ctx).await,
            "list" => self.handle_list().await,
            _ => Err(PluginError::NotFound(format!("Unknown path: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_HOOK, HooksPlugin::build, dyn Plugin);
