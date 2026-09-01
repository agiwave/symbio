//! Web Tools 插件实现

use super::{http_request::HttpRequestTool, web_fetch::WebFetchTool, web_search::WebSearchTool};
use crate::symbio_core::schemas::common::SimpleResponse;
pub use crate::symbio_core::schemas::web::web_config::WebConfig;
use crate::symbio_core::{
    Capability, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_WEB,
};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct WebPlugin {
    config: Arc<RwLock<WebConfig>>,
    tool_impls: Arc<Vec<Arc<dyn Capability>>>,
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl WebPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config: WebConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        Arc::new(WebPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    pub fn new(parent: Option<Weak<dyn Plugin>>, config: WebConfig) -> Self {
        let config_lock = Arc::new(RwLock::new(config));

        let web_fetch = Arc::new(WebFetchTool::new());
        let web_search = Arc::new(WebSearchTool::new(Arc::clone(&config_lock)));
        let http_request = Arc::new(HttpRequestTool::new());

        let tool_impls: Vec<Arc<dyn Capability>> = vec![web_fetch, web_search, http_request];

        Self {
            config: config_lock,
            tool_impls: Arc::new(tool_impls),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("web", "网络工具集")
            .with_description("提供 Web 搜索、网络请求等网络相关工具")
            .with_version("0.1.0")
    }
}

#[async_trait]
impl Plugin for WebPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();

        if path.starts_with('/') {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        match path.as_str() {
            CONFIG_GET => {
                let cfg = self.config.read().await;
                Ok(PluginPayload::new(&*cfg))
            }
            CONFIG_SET => {
                let new_cfg: WebConfig = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = new_cfg.clone();
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    let _ = p.route(save_ctx).await;
                }
                Ok(PluginPayload::new(&SimpleResponse::success()))
            }
            _ => {
                if let Some(tool) = self.tool_impls.iter().find(|t| t.name() == path) {
                    return tool.execute(ctx).await;
                }
                Err(PluginError::NotFound(format!("路径不存在: {path}")))
            }
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            for tool in self.tool_impls.iter() {
                tool_manager.register(tool.clone()).await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_WEB, WebPlugin::build, dyn Plugin);
