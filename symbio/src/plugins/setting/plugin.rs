//! Setting 插件 - 设置管理

use crate::symbio_core::schemas::SuccessResponse;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_SETTING,
};
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use crate::symbio_core::schemas::setting::{setting_get, setting_list};
use tracing::info;

#[derive(Clone)]
pub struct SettingPlugin {
    config: Arc<RwLock<Value>>,
    /// 父插件引用
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl SettingPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config = ctx.config().unwrap_or_else(|| serde_json::json!({}));
        let parent = ctx.parent();

        Arc::new(SettingPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: Value) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("setting", "系统设置")
            .with_description("设置管理插件")
            .with_version("0.1.0")
    }

    /// 获取父插件引用
    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }
}

impl Default for SettingPlugin {
    fn default() -> Self {
        Self::new(None, json!({}))
    }
}

#[async_trait::async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "list" => {
                let categories = vec![
                    setting_list::SettingCategory {
                        id: "general".to_string(),
                        name: "常规设置".to_string(),
                        icon: "settings".to_string(),
                    },
                    setting_list::SettingCategory {
                        id: "model".to_string(),
                        name: "Model 设置".to_string(),
                        icon: "smart_toy".to_string(),
                    },
                ];
                Ok(PluginPayload::new(&setting_list::Response { categories }))
            }
            "get" => {
                let req: setting_get::Request = ctx.payload()?;

                let cfg = self.config.read().await;
                // 尝试从配置中按分类提取，如果没有该分类，则返回空对象
                let category_settings =
                    cfg.get(&req.category).cloned().unwrap_or_else(|| json!({}));

                info!(
                    category = %req.category,
                    keys = ?category_settings.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                    "获取分类设置"
                );

                Ok(PluginPayload::new(&setting_get::Response {
                    category: req.category.clone(),
                    settings: category_settings,
                }))
            }
            CONFIG_GET => {
                let cfg = self.config.read().await;
                // 彻底扁平化：直接返回 Value 对象，不再包装在 Response.config 中
                let cfg_value = cfg.clone();
                Ok(PluginPayload::new(&cfg_value))
            }
            CONFIG_SET => {
                let payload: serde_json::Value = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = payload.clone();
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    p.route(save_ctx).await?;
                }
                Ok(PluginPayload::new(&SuccessResponse::default()))
            }
            _ => Err(PluginError::NotFound(format!("未知路径: {path}"))),
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

crate::submit_object_creator!(PLUGIN_SETTING, SettingPlugin::build, dyn Plugin);
