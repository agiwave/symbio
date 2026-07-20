//! Home 插件 - 根插件，持有并调度所有顶级子插件 (work, agent, setting, explorer 等)
//!
//! 采用分形路由架构：
//! - 负责全局配置的持久化
//! - 负责绝对路径路由的终结处理
//! - 递归聚合所有子插件的工具能力
//!
//! ## 系统目录 (homedir)
//!
//! Home 插件的配置文件位于 [`HomedirRegistry::get()`] / `config.yaml`，
//! 即当前系统目录。homedir 切换通过 `home/reload` 路由热重载实现。

use crate::symbio_core::schemas::{common, explorer::home_reload, work::work_get_workspace};
use crate::symbio_core::{
    HomedirRegistry, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError,
    PluginMeta, PluginPayload, SimpleRequest, CONFIG_GET, PATH, PLUGIN_COMPOSITE, PLUGIN_HOME,
};
use crate::{plugin_debug, plugin_error, plugin_info, plugin_warn};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::info;

/// 配置文件路径：基于当前 homedir
pub(crate) fn config_path() -> PathBuf {
    HomedirRegistry::get().join("config.yaml")
}

/// 全局配置结构
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub symbio: serde_json::Map<String, Value>,
}

impl GlobalConfig {
    pub fn ensure_defaults(&mut self) {
        // 确保 plugins 节点存在
        let plugins = self
            .symbio
            .entry("plugins".to_string())
            .or_insert_with(|| serde_json::json!({}));

        if let Some(obj) = plugins.as_object_mut() {
            // 默认插件列表
            let defaults = [
                "setting",
                "event_bus",
                "explorer",
                "model",
                "session",
                "local",
                "web",
                "mcp",
                "telegram",
                "hook",
                "agent",
                "skill",
            ];
            for name in &defaults {
                obj.entry(name.to_string()).or_insert_with(|| {
                    serde_json::json!({
                        "plugin_name": name,
                        "plugin_provider": name
                    })
                });
            }
        }
    }
}

pub struct HomePlugin {
    /// 子插件实例容器 (如 "work" -> AgentPlugin)
    pub instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
    /// 全局配置缓存
    config: Arc<RwLock<GlobalConfig>>,
    /// 插件上下文（用于动态创建子插件）
    context: Arc<dyn InvokeRequest>,
    /// 自身的弱引用
    self_weak: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl HomePlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let mut global_config: GlobalConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        if global_config.symbio.is_empty() {
            let path = config_path();
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(cfg) = serde_yml::from_str::<GlobalConfig>(&content) {
                        info!(path = %path.display(), "从磁盘成功加载配置");
                        global_config = cfg;
                    }
                }
            }
        }

        global_config.ensure_defaults();

        let home = Arc::new(Self::new_with_config(global_config.clone(), ctx.clone()));
        home.set_self_sync(Arc::downgrade(&home) as Weak<dyn Plugin>);

        if let Err(e) = home.init_worker_composite_sync() {
            plugin_warn!("home", "Failed to initialize worker composite: {}", e);
        }

        if let Some(worker_cfg) = global_config.symbio.get("work") {
            if let Some(path) = worker_cfg.get("workdir").and_then(|v| v.as_str()) {
                if !path.is_empty() {
                    let home_clone = Arc::clone(&home);
                    let path_to_restore = path.to_string();

                    tokio::spawn(async move {
                        plugin_info!("home", "正在自动恢复上次的工作区: {}", path_to_restore);
                        if home_clone.set_workspace(&path_to_restore).await.is_ok() {
                            let _ = home_clone.save_config().await;
                        }
                    });
                }
            }
        }

        home as Arc<dyn Plugin>
    }

    pub fn new(context: Arc<dyn InvokeRequest>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(GlobalConfig::default())),
            context,
            self_weak: Arc::new(RwLock::new(None)),
        }
    }

    /// 带初始配置的构造函数（工厂使用）
    pub fn new_with_config(config: GlobalConfig, context: Arc<dyn InvokeRequest>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
            context,
            self_weak: Arc::new(RwLock::new(None)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("home", "Home")
            .with_description("Symbio 主插件")
            .with_version("0.1.0")
    }

    /// 热重载：用于 homedir 切换或 bootstrap 恢复后的整体重建
    ///
    /// 流程：
    /// 1. （可选）切换 homedir：先持久化到 bootstrap，再更新内存
    /// 2. 把当前 config 持久化到**旧** homedir（如果换了的话）
    /// 3. 从**新** homedir 重新读 config
    /// 4. 清空所有子插件实例
    /// 5. 重建 worker composite（子插件会从新 homedir 加载数据）
    ///
    /// 注意：本方法**不**异步恢复 workdir（需要 Arc<Self> 才能 spawn task），
    /// 由 route handler 在 reload 完成后自行 spawn。
    ///
    /// 调用方应**先**关闭所有活跃 chat 会话（`RouteConnectionManager::remove_all`），
    /// 否则旧 session 上的 PluginChannel 持有的旧 SessionPlugin 引用会变成"幽灵"。
    pub async fn reload(
        &self,
        new_homedir: Option<PathBuf>,
    ) -> Result<home_reload::Response, PluginError> {
        let old_homedir = HomedirRegistry::get();
        let mut homedir_changed = false;

        // 1. 切换 homedir（如有）
        if let Some(target) = new_homedir {
            if target != old_homedir {
                HomedirRegistry::set(target.clone())
                    .map_err(|e| PluginError::InternalError(format!("切换 homedir 失败: {e}")))?;
                homedir_changed = true;
                plugin_info!(
                    "home",
                    "homedir 已切换: {} -> {}",
                    old_homedir.display(),
                    target.display()
                );
            }
        }

        let new_homedir = HomedirRegistry::get();

        // 2. 把旧 config 写回旧 homedir（仅在 homedir_changed 时）
        if homedir_changed {
            if let Err(e) = self.save_config().await {
                plugin_warn!(
                    "home",
                    "homedir 切换前持久化旧 config 失败（忽略继续）: {}",
                    e
                );
            }
        }

        // 3. 从新 homedir 读 config
        let mut new_config: GlobalConfig = {
            let path = config_path(); // 注意：这里用新 homedir，因为上面已 set
            if path.exists() {
                std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| serde_yml::from_str::<GlobalConfig>(&content).ok())
                    .unwrap_or_default()
            } else {
                GlobalConfig::default()
            }
        };
        new_config.ensure_defaults();

        // 4. 清空所有子插件实例（持有锁期间不会有人访问）
        {
            let mut instances = self.instances.write().await;
            instances.clear();
        }

        // 5. 替换 config 缓存
        {
            let mut cfg = self.config.write().await;
            *cfg = new_config.clone();
        }

        // 6. 重建 worker composite
        if let Err(e) = self.rebuild_worker_sync() {
            return Err(PluginError::InternalError(format!(
                "重建 worker composite 失败: {e}"
            )));
        }
        let reloaded_plugins = {
            let instances = self.instances.read().await;
            instances.get("worker").map(|_| 1).unwrap_or(0)
        };

        plugin_info!(
            "home",
            "reload 完成: old={}, new={}, changed={}",
            old_homedir.display(),
            new_homedir.display(),
            homedir_changed
        );

        Ok(home_reload::Response {
            old_homedir: old_homedir.to_string_lossy().to_string(),
            new_homedir: new_homedir.to_string_lossy().to_string(),
            reloaded_plugins,
            homedir_changed,
            bootstrap_path: HomedirRegistry::bootstrap_path_display(),
        })
    }

    /// 获取当前 homedir 信息
    pub async fn get_homedir_info(&self) -> Value {
        serde_json::json!({
            "homedir": HomedirRegistry::get().to_string_lossy().to_string(),
            "bootstrap_path": HomedirRegistry::bootstrap_path_display(),
            "default_homedir": {
                "path": HomedirRegistry::get().to_string_lossy().to_string(),
            }
        })
    }

    /// 清理已迁到新存储（`~/.symbio/plugins/`）的插件数据
    ///
    /// **策略**：
    /// - `model`（原 `ai`）：删除 `providers` 字段，保留 `default_provider_id` 和 `_storage` 标记
    /// - `mcp`：删除 `servers` 字段
    ///
    /// 这样 config.yaml 不再包含已迁移的实际数据，避免下次启动时
    /// 被读取到"陈旧副本"（与新存储中的真实数据不一致）。
    ///
    /// **AI → model 兼容**：若旧 config 仍以 `ai` 为 key，会先迁移到 `model` 再清理。
    fn prune_migrated_plugin_data(symbio_cfg: &mut serde_json::Map<String, Value>) {
        // 兼容：旧 config 可能仍以 "ai" 为 key —— 迁移到 "model"
        if !symbio_cfg.contains_key("model") {
            if let Some(ai_val) = symbio_cfg.remove("ai") {
                symbio_cfg.insert("model".to_string(), ai_val);
            }
        }
        if let Some(model_val) = symbio_cfg.get_mut("model") {
            if let Some(model_obj) = model_val.as_object_mut() {
                model_obj.remove("providers");
            }
        }
        if let Some(mcp_val) = symbio_cfg.get_mut("mcp") {
            if let Some(mcp_obj) = mcp_val.as_object_mut() {
                mcp_obj.remove("servers");
            }
        }
    }

    /// 同步版 `set_self`：构造期单线程写入，使用 `try_write` 不会失败
    pub fn set_self_sync(&self, weak: Weak<dyn Plugin>) {
        *self
            .self_weak
            .try_write()
            .expect("HomePlugin::set_self_sync: self_weak lock contended during construction") =
            Some(weak);
    }

    /// 同步版 `add_instance`：构造期单线程写入，使用 `try_write` 不会失败
    pub fn add_instance_sync(&self, name: String, plugin: Arc<dyn Plugin>) {
        let mut guard = self
            .instances
            .try_write()
            .expect("HomePlugin::add_instance_sync: instances lock contended during construction");
        guard.insert(name, plugin);
    }

    /// 同步版 `init_worker_composite`：供同步构造函数调用
    /// 构造期单线程，使用 `try_read` / `try_write` 不会失败
    pub fn init_worker_composite_sync(&self) -> Result<(), PluginError> {
        self.rebuild_worker_sync()
    }

    /// 重建 worker composite (用于初始化和 reload)
    ///
    /// 同步版本，可在构造期和 reload 流程中复用。
    /// - 构造期：self_weak 已被 set_self_sync 设置，composite 不存在
    /// - reload：composite 已存在，需要先 remove 再 add（不可重复 add 同名）
    pub fn rebuild_worker_sync(&self) -> Result<(), PluginError> {
        use crate::symbio_core::has_creator;
        if !has_creator(PLUGIN_COMPOSITE) {
            return Ok(());
        }

        let sub_config = {
            let cfg = self
                .config
                .try_read()
                .expect("HomePlugin::rebuild_worker_sync: config lock contended");
            let val = cfg.symbio.get("plugins").cloned();
            if let Some(ref v) = val {
                plugin_info!(
                    "home",
                    "成功为 'plugins' 实例匹配到持久化配置 (Length: {})",
                    v.to_string().len()
                );
            } else {
                plugin_warn!("home", "未找到 'plugins' 的持久化配置，将使用默认值");
            }
            val
        };

        // 创建子上下文（self_weak 在 set_self_sync 之后一定存在）
        let self_weak = self
            .self_weak
            .try_read()
            .expect("HomePlugin::rebuild_worker_sync: self_weak lock contended")
            .clone()
            .expect("HomePlugin self_weak not set");

        let sub_context = Arc::new(SimpleRequest::new(Some(self_weak), sub_config));

        // 继承父上下文的环境变量
        if let Some(std_ctx) = self.context.as_any().downcast_ref::<SimpleRequest>() {
            if let Some(sub_std_ctx) = sub_context.as_any().downcast_ref::<SimpleRequest>() {
                let mut sub_envs = sub_std_ctx.envs.write().unwrap();
                *sub_envs = std_ctx.envs.read().unwrap().clone();
            }
        }

        let worker_plugin: Arc<dyn Plugin> =
            crate::symbio_core::create_object::<dyn Plugin>("composite", sub_context)
                .expect("composite creator registered but failed to construct");
        self.add_instance_sync("worker".to_string(), worker_plugin);
        plugin_info!(
            "home",
            "工作区插件 'worker' (Composite) 构造完成并已挂载 (Stateless Singleton)"
        );
        Ok(())
    }

    /// 切换工作区：仅更新配置和最近使用记录
    pub async fn set_workspace(&self, path_str: &str) -> Result<Value, PluginError> {
        let expanded_path = shellexpand::tilde(path_str).to_string();

        plugin_info!("home", "正在切换到工作区: {}", expanded_path);

        // 2. 更新并保存配置
        let recents = {
            let mut cfg = self.config.write().await;
            let work_entry = cfg
                .symbio
                .entry("work".to_string())
                .or_insert_with(|| serde_json::json!({}));

            if let Some(obj) = work_entry.as_object_mut() {
                obj.insert("workdir".to_string(), serde_json::json!(path_str));

                // 更新最近工作区
                let mut recents = obj
                    .get("recent_workspaces")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                let current_path = serde_json::json!(path_str);
                recents.retain(|v| v != &current_path);
                recents.insert(0, current_path);
                recents.truncate(10);

                let recents_val = Value::Array(recents.clone());
                obj.insert("recent_workspaces".to_string(), recents_val);

                recents
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            } else {
                Vec::new()
            }
        };

        Ok(serde_json::json!({
            "workdir": path_str,
            "expanded_path": expanded_path,
            "recent_workspaces": recents,
            "status": "success"
        }))
    }

    /// 保存配置到文件（原子化保存）
    pub async fn save_config(&self) -> Result<(), PluginError> {
        plugin_debug!("home", "开始收集活跃插件配置...");
        let collected = self.collect_plugin_configs().await;

        {
            let mut cfg = self.config.write().await;
            // 增量合并，不直接覆盖整个插件节点，而是合并对象属性
            for (name, new_val) in collected {
                let existing_val = cfg
                    .symbio
                    .entry(name.clone())
                    .or_insert_with(|| serde_json::json!({}));

                if let (Some(existing_obj), Some(new_obj)) =
                    (existing_val.as_object_mut(), new_val.as_object())
                {
                    // 将新获取的配置合并到现有配置中
                    for (k, v) in new_obj {
                        existing_obj.insert(k.clone(), v.clone());
                    }
                } else {
                    // 如果不是对象，则直接覆盖
                    cfg.symbio.insert(name, new_val);
                }
            }

            // ⭐ 清理已迁到新存储的插件的实际数据字段
            // model（原 ai）：实际 providers 数据在 ~/.symbio/plugins/model/<id>/provider.json
            // mcp：实际 servers 数据在 ~/.symbio/plugins/mcp/<name>/server.json
            // 保留元数据（如 default_provider_id）以兼容旧读取路径
            Self::prune_migrated_plugin_data(&mut cfg.symbio);
        }

        let path = config_path();
        let tmp_path = path.with_extension("yaml.tmp");

        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                plugin_error!(
                    "home",
                    format!("创建配置目录失败: {} (路径: {})", e, parent.display())
                );
                return Err(PluginError::InternalError(format!("创建目录失败: {e}")));
            }
        }

        let cfg = self.config.read().await;
        let content = match serde_yml::to_string(&*cfg) {
            Ok(c) => c,
            Err(e) => {
                plugin_error!("home", format!("序列化 YAML 失败: {}", e));
                return Err(PluginError::InternalError(format!("序列化配置失败: {e}")));
            },
        };

        // 1. 先写入临时文件
        if let Err(e) = tokio::fs::write(&tmp_path, content).await {
            plugin_error!(
                "home",
                format!("写入临时配置失败: {} (路径: {})", e, tmp_path.display())
            );
            return Err(PluginError::InternalError(format!("写入临时文件失败: {e}")));
        }

        // 2. 强刷磁盘确保写入完成（可选，但更安全）
        // 3. 原子化重命名
        if let Err(e) = tokio::fs::rename(&tmp_path, &path).await {
            plugin_error!(
                "home",
                format!(
                    "重命名配置失败: {} ({} -> {})",
                    e,
                    tmp_path.display(),
                    path.display()
                )
            );
            return Err(PluginError::InternalError(format!("重命名失败: {e}")));
        }

        plugin_info!(
            "home",
            "活跃插件配置已成功原子化持久化至: {}",
            path.display()
        );
        Ok(())
    }

    /// 收集所有子插件配置
    async fn collect_plugin_configs(&self) -> serde_json::Map<String, Value> {
        let mut configs = serde_json::Map::new();

        let worker_opt = {
            let instances = self.instances.read().await;
            instances.get("worker").cloned()
        };

        if let Some(worker) = worker_opt {
            let ctx = Arc::new(SimpleRequest::new(None, None));
            ctx.set(PATH, CONFIG_GET.to_string());

            match worker.clone().route(ctx).await {
                Ok(resp) => {
                    if let Ok(data) = resp.get::<serde_json::Value>() {
                        configs.insert("plugins".to_string(), data);
                    }
                },
                Err(e) => {
                    plugin_error!("home", format!("获取插件 'worker' 配置失败: {}", e));
                },
            }
        }
        configs
    }

    /// 解析路径，返回 (子插件名, 剩余路径)
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

impl Default for HomePlugin {
    fn default() -> Self {
        Self::new(Arc::new(SimpleRequest::new(None, None)))
    }
}

crate::submit_object_creator!(PLUGIN_HOME, HomePlugin::build, dyn Plugin);

#[async_trait::async_trait]
impl Plugin for HomePlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "save_config" => {
                let this = Arc::clone(&self);
                tokio::spawn(async move {
                    if let Err(e) = this.save_config().await {
                        plugin_error!("home", format!("异步持久化任务失败: {}", e));
                    }
                });
                return Ok(PluginPayload::new(&common::SuccessResponse::default()));
            },
            "home/reload" => {
                let req: home_reload::Request = ctx.payload().unwrap_or_default();
                let new_homedir = req.homedir.as_ref().map(PathBuf::from);
                let result = self.reload(new_homedir).await?;

                // 恢复 workdir (需要 Arc<Self> 才能 spawn)
                let workdir_to_restore = {
                    let cfg = self.config.read().await;
                    cfg.symbio
                        .get("work")
                        .and_then(|w| w.get("workdir"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                };
                if !workdir_to_restore.is_empty() {
                    let home_arc = Arc::clone(&self);
                    let path_to_restore = workdir_to_restore;
                    tokio::spawn(async move {
                        plugin_info!("home", "reload: 正在恢复 workdir: {}", path_to_restore);
                        if home_arc.set_workspace(&path_to_restore).await.is_ok() {
                            let _ = home_arc.save_config().await;
                        }
                    });
                }

                return Ok(PluginPayload::new(&result));
            },
            "home/get_homedir" => {
                let info = self.get_homedir_info().await;
                return Ok(PluginPayload::new(&info));
            },
            "work/set_workspace" => {
                #[derive(serde::Deserialize, Clone)]
                struct SetWorkspaceRequest {
                    path: String,
                }
                let req: SetWorkspaceRequest = ctx.payload()?;
                let path_str = &req.path;

                let result = self.set_workspace(path_str).await?;

                // 切换成功后，异步触发一次配置持久化
                let this = Arc::clone(&self);
                tokio::spawn(async move {
                    if let Err(e) = this.save_config().await {
                        plugin_error!("home", format!("切换工作区后自动持久化失败: {}", e));
                    }
                });

                return Ok(PluginPayload::new(&result));
            },
            "work/get_workspace" => {
                // 统一从 Home 的配置缓存中读取工作区路径，这是最可靠的数据源
                let cfg = self.config.read().await;
                let work_cfg = cfg.symbio.get("work").cloned().unwrap_or_default();
                let wp = work_cfg
                    .get("workdir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let recents = work_cfg
                    .get("recent_workspaces")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_else(Vec::new);

                return Ok(PluginPayload::new(&work_get_workspace::Response {
                    workdir: wp.to_string(),
                    expanded_path: if wp.is_empty() {
                        "".to_string()
                    } else {
                        shellexpand::tilde(wp).to_string()
                    },
                    recent_workspaces: recents,
                }));
            },
            _ => {},
        }

        if let Some((name, rest)) = Self::parse_path(path) {
            let plugin_opt = {
                let instances = self.instances.read().await;
                instances.get(name).cloned()
            };

            if let Some(plugin) = plugin_opt {
                // 修改上下文路径并转发
                let child_ctx = ctx.fork();
                child_ctx.set(PATH, rest.to_string());
                return plugin.route(child_ctx).await;
            }
        }

        // --- 核心改动：对于不认识的路由，转交给 work (Agent) 处理 ---
        let worker_opt = {
            let instances = self.instances.read().await;
            instances.get("worker").cloned()
        };

        if let Some(worker) = worker_opt {
            // 注意：不剥离路径，直接将原始请求转发给 Agent
            return worker.route(ctx).await;
        }

        Err(PluginError::NotFound(format!(
            "Home: 路径 '{path}' 无法识别且无工作区运行"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let mut results = Vec::new();
        let instances = {
            let guard = self.instances.read().await;
            guard
                .iter()
                .map(|(n, p)| (n.clone(), Arc::clone(p)))
                .collect::<Vec<_>>()
        };

        for (name, plugin) in instances {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };

            let req_ctx = ctx.fork();
            if let Ok(res) = plugin.traverse(child_path, req_ctx).await {
                if let Ok(val) = res.get::<serde_json::Value>() {
                    if let Some(arr) = val.as_array() {
                        results.extend(arr.clone());
                    } else {
                        results.push(val);
                    }
                }
            }
        }

        Ok(PluginPayload::new(&results))
    }
}

impl Clone for HomePlugin {
    fn clone(&self) -> Self {
        Self {
            instances: Arc::clone(&self.instances),
            config: Arc::clone(&self.config),
            context: self.context.clone(),
            self_weak: Arc::clone(&self.self_weak),
        }
    }
}
