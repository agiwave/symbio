//! Composite 插件实现——结构体 + Plugin trait impl
//!
//! 工厂逻辑通过 `Composite::build` 静态方法 + `submit_object_creator!` 自注册。

use crate::symbio_core::{
    create_object, has_creator, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginError, PluginMeta, PluginPayload, SimpleRequest, CONFIG_GET, PATH, PLUGIN_COMPOSITE,
};

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::{info, warn};

pub struct Composite {
    instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
    /// 父插件引用
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    /// 环境变量（层层透传）
    envs: HashMap<String, String>,
}

impl Composite {
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            parent: Arc::new(RwLock::new(parent)),
            envs: HashMap::new(),
        }
    }

    pub fn new_with_envs(parent: Option<Weak<dyn Plugin>>, envs: HashMap<String, String>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            parent: Arc::new(RwLock::new(parent)),
            envs,
        }
    }

    /// 同步版 `add_instance`，供构造函数在同步上下文中调用
    /// 构造期（`init()` / `create_root_plugin()`）单线程写入，使用 `try_write` 不会失败
    pub fn add_instance_sync(&self, name: String, plugin: Arc<dyn Plugin>) {
        let mut guard = self
            .instances
            .try_write()
            .expect("Composite::add_instance_sync: instances lock contended during construction");
        guard.insert(name, plugin);
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    fn parse_path(path: &str) -> Option<(&str, &str)> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return None;
        }
        match path.find('/') {
            Some(idx) => Some((&path[..idx], &path[idx + 1..])),
            None => Some((path, "")),
        }
    }
}

impl Default for Composite {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Clone for Composite {
    fn clone(&self) -> Self {
        Self {
            instances: Arc::clone(&self.instances),
            parent: Arc::clone(&self.parent),
            envs: self.envs.clone(),
        }
    }
}

impl Composite {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let composite_config = ctx.config();

        if let Some(cfg) = &composite_config {
            info!(
                keys = ?cfg.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                "composite_factory 收到持久化配置，准备分发给子插件"
            );
        } else {
            warn!("composite_factory 未收到有效配置，子插件将使用默认值");
        }

        let envs = if let Some(std_ctx) = ctx.as_any().downcast_ref::<SimpleRequest>() {
            std_ctx.envs.read().unwrap().clone()
        } else {
            HashMap::new()
        };

        let parent = ctx.parent();

        let composite = Arc::new(Self::new_with_envs(parent, envs));
        let composite_weak = Arc::downgrade(&composite) as Weak<dyn Plugin>;

        if let Some(cfg_obj) = composite_config.as_ref().and_then(|v| v.as_object()) {
            for (name, sub_val) in cfg_obj {
                let plugin_provider_name = sub_val
                    .get("plugin_provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name);

                if !has_creator(plugin_provider_name) {
                    warn!(
                        name,
                        plugin_provider_name, "composite_factory 跳过子插件：未找到 Provider"
                    );
                    continue;
                }

                let sub_config = sub_val.clone();
                let sub_context = Arc::new(SimpleRequest::new(
                    Some(composite_weak.clone()),
                    Some(sub_config),
                ));

                if let Some(std_ctx) = ctx.as_any().downcast_ref::<SimpleRequest>() {
                    let mut sub_envs = sub_context.envs.write().unwrap();
                    *sub_envs = std_ctx.envs.read().unwrap().clone();
                }

                info!(
                    name,
                    plugin_provider_name, "composite_factory -> 正在构造子插件"
                );

                if let Some(plugin_instance) =
                    create_object::<dyn Plugin>(plugin_provider_name, sub_context)
                {
                    composite.add_instance_sync(name.to_string(), plugin_instance);
                } else {
                    warn!(
                        name,
                        plugin_provider_name, "composite_factory 子插件 Provider 构造失败"
                    );
                }
            }
        }

        composite as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("composite", "通用插件容器")
            .with_description("通用的插件容器，可以管理子插件实例，支持嵌套")
            .with_version("0.1.0")
    }
}

crate::submit_object_creator!(PLUGIN_COMPOSITE, Composite::build, dyn Plugin);

#[async_trait::async_trait]
impl Plugin for Composite {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 1. 绝对路径重定向：如果路径收到 save_config/load_config，直接转发给防节点
        if path == "save_config" {
            if let Some(parent) = self.get_parent().await {
                return parent.route(ctx).await;
            }
        }

        // 2. 配置聚合
        if path == CONFIG_GET {
            let mut configs = serde_json::Map::new();
            let plugins: Vec<(String, Arc<dyn Plugin>)> = {
                let instances = self.instances.read().await;
                instances
                    .iter()
                    .map(|(name, plugin)| (name.clone(), Arc::clone(plugin)))
                    .collect()
            };
            for (name, plugin) in plugins {
                let sub_ctx = ctx.fork();
                sub_ctx.set(PATH, CONFIG_GET.to_string());

                if let Ok(payload) = plugin.clone().route(sub_ctx).await {
                    if let Ok(mut data) = payload.get::<Value>() {
                        // 注入插件 provider 信息 (从插件 meta 中获取)
                        if let Some(obj) = data.as_object_mut() {
                            obj.insert(
                                "plugin_provider".to_string(),
                                serde_json::json!(plugin.meta().id),
                            );
                            // 同时也确保 plugin_name 存在 (可选，但推荐)
                            if !obj.contains_key("plugin_name") {
                                obj.insert("plugin_name".to_string(), serde_json::json!(name));
                            }
                        }
                        configs.insert(name, data);
                    }
                }
            }
            return Ok(PluginPayload::new(&Value::Object(configs)));
        }

        // 4. 子插件分发
        if let Some((name, rest)) = Self::parse_path(path) {
            let plugin_opt = {
                let instances = self.instances.read().await;
                instances.get(name).cloned()
            };

            if let Some(plugin) = plugin_opt {
                let child_ctx = ctx.fork();
                child_ctx.set(PATH, rest.to_string());
                return plugin.route(child_ctx).await;
            }
        }

        Err(PluginError::NotFound(format!(
            "Composite: 路径 '{path}' 无法识别或子插件未挂载"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let instances: Vec<(String, Arc<dyn Plugin>)> = {
            let guard = self.instances.read().await;
            guard
                .iter()
                .map(|(n, p)| (n.clone(), Arc::clone(p)))
                .collect()
        };

        for (_name, plugin) in instances {
            let req_ctx = ctx.fork();
            let _ = plugin.traverse("".to_string(), req_ctx).await;
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
