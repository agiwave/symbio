//! Home 插件 - 根插件，持有并调度所有顶级子插件 (work, agent, plugin_manager 等)
//!
//! 采用分形路由架构：
//! - 负责绝对路径路由的终结处理
//! - 递归聚合所有子插件的工具能力
//!
//! ## 系统目录 (homedir)
//!
//! Home 是**系统根插件**：它的目录就是系统根 [`HomedirRegistry::get()`] 本身，
//! 配置在 `<homedir>/PLUGIN.yml`——与其它插件**同一套规范**
//! （见 [`plugin_dir`](crate::symbio_core::plugin::dir)），只是它住在系统根而不是
//! 业务插件**并列**在系统根下（它管辖的插件根就是系统根本身）。
//! homedir 切换通过 `home/reload` 路由热重载实现。
//!
//! 它构造的容器 `composite` 是它的**动态内置替身**，共用同一个系统根目录。
//!
//! ## 它不管任何子插件的配置
//!
//! 配置属于**拥有者**：每个插件目录里的 `PLUGIN.yml`，谁写谁读。本插件只保留
//! 自己的应用级状态（工作区 / 最近记录）。「一个插件的配置」因此只有一个落位，
//! 不存在集中合并这一层。

use super::homedir::HomedirRegistry;
use super::schemas::{home_reload, work_get_workspace};
use crate::symbio_core::{
    Plugin, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest, PATH, PLUGIN_DIR,
    PLUGIN_ID_COMPOSITE, PLUGIN_ID_HOME, REQUIRED_PLUGINS,
};
use crate::{plugin_error, plugin_info, plugin_warn};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// 系统必备插件：**home 的策略**，随构造交给容器。
///
/// 系统（根）Agent 挂载的插件清单。**当前与子 Agent 子树逐项相同**，故直接取
/// `symbio_core` 的 [`ASSEMBLY_SUB_AGENT_PLUGINS`](crate::symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS)
/// ——这里**不是**第二份手抄的字面量。父子因此「结构一致、能力对齐」；二者的差异在
/// **收集期作用域**（`vdfs` 单槽归根、`model` 单槽按收集方）而非清单内容——详见
/// `ASSEMBLY_SUB_AGENT_PLUGINS` 的文档。
///
/// 若将来两侧确需分叉，**在这里加只属于系统侧的字面量并写清理由**——不要恢复
/// 「两张各写一遍、靠人同步」的形态（两份各自演化的清单会静默漂移，且没有测试会变红）。
pub const SYSTEM_PLUGINS: &[&str] = crate::symbio_core::ASSEMBLY_SUB_AGENT_PLUGINS;

/// Home 自己的插件目录 = **系统根** `<homedir>`
///
/// 不落在插件根下：若落在那里（即系统根下再有一层 `home/`），容器扫描插件根时会把它当普通插件再构造一次，
/// 那个 home 又去构造容器——自举环。系统级插件不参与扫描。
fn home_dir() -> PluginDir {
    // home 是**唯一**知道 homedir 的插件：它的目录就是系统根。
    PluginDir::at(HomedirRegistry::get(), PLUGIN_ID_HOME)
}

/// Home 自己的配置（`<homedir>/PLUGIN.yml`）
///
/// 只有**应用级状态**：工作区与最近记录。插件配置不在这里——每个插件配置在
/// 自己的目录里（`plugin_provider` / `plugin_name` 两个身份字段由
/// [`PluginDir`] 读写时自动剥离 / 补回）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct HomeConfig {
    /// 工作区状态：`workdir` / `recent_workspaces`
    #[serde(default)]
    pub work: serde_json::Map<String, Value>,
}

impl HomeConfig {
    /// 确保 `work` 节点存在（首次启动时给一个可写的空壳）
    pub fn ensure_defaults(&mut self) {
        self.work
            .entry("workdir".to_string())
            .or_insert_with(|| Value::String(String::new()));
        self.work
            .entry("recent_workspaces".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
}

pub struct HomePlugin {
    /// 子插件实例容器 (如 "work" -> AgentPlugin)
    pub instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
    /// 自己的配置缓存（`<本插件目录>/PLUGIN.yml`）
    config: Arc<RwLock<HomeConfig>>,
    /// 插件上下文（用于动态创建子插件）
    context: Arc<dyn PluginInvokeRequest>,
    /// 自身的弱引用
    self_weak: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl HomePlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = home_dir();
        if let Err(e) = dir.ensure_manifest() {
            plugin_warn!("home", "补建自身插件目录失败：{}", e);
        }

        // 1. 读自己的配置
        let mut config: HomeConfig = match dir.load::<HomeConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => HomeConfig::default(),
            Err(e) => {
                plugin_error!("home", "读取自身配置失败，改用默认值：{}", e);
                HomeConfig::default()
            }
        };
        config.ensure_defaults();

        let home = Arc::new(Self::new_with_config(config.clone(), ctx.clone()));
        home.set_self_sync(Arc::downgrade(&home) as Weak<dyn Plugin>);

        if let Err(e) = home.init_worker_composite_sync() {
            plugin_warn!("home", "Failed to initialize worker composite: {}", e);
        }

        if let Some(path) = config.work.get("workdir").and_then(Value::as_str) {
            if !path.is_empty() {
                let home_clone = Arc::clone(&home);
                let path_to_restore = path.to_string();

                tokio::spawn(async move {
                    plugin_info!("home", "正在自动恢复上次的工作区: {}", path_to_restore);
                    if home_clone.set_workspace(&path_to_restore).await.is_ok() {
                        // flush 失败 = 刚恢复出来的工作区**没落盘**，下次启动会再丢一次。
                        // 这条路径在后台 spawn 里跑，没人看得到返回值，故必须自己留痕。
                        if let Err(e) = home_clone.flush().await {
                            plugin_warn!("home", "自动恢复工作区后 flush 失败：{}", e);
                        }
                    }
                });
            }
        }

        home as Arc<dyn Plugin>
    }

    pub fn new(context: Arc<dyn PluginInvokeRequest>) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(HomeConfig::default())),
            context,
            self_weak: Arc::new(RwLock::new(None)),
        }
    }

    /// 带初始配置的构造函数（工厂使用）
    pub fn new_with_config(config: HomeConfig, context: Arc<dyn PluginInvokeRequest>) -> Self {
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
            if let Err(e) = self.flush().await {
                plugin_warn!(
                    "home",
                    "homedir 切换前持久化旧 config 失败（忽略继续）: {}",
                    e
                );
            }
        }

        // 3. 从新 homedir 读 config（路径现取，因为上面已 set）
        let dir = home_dir();
        let mut new_config: HomeConfig = match dir.load::<HomeConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => HomeConfig::default(),
            Err(e) => {
                plugin_error!("home", "切换 homedir 后读取自身配置失败，改用默认值：{}", e);
                HomeConfig::default()
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
    ///
    /// **不再传任何配置**：容器的子项来自插件目录扫描（见 `composite`），
    /// 每个插件从自己的目录读配置。容器是 home 的**动态内置替身**，因此拿到的
    /// 也是系统根目录——它据此定位插件根 `<系统根>/plugins`。
    pub fn rebuild_worker_sync(&self) -> Result<(), PluginError> {
        use crate::symbio_core::has_creator;
        if !has_creator(PLUGIN_ID_COMPOSITE) {
            return Ok(());
        }

        // 创建子上下文（self_weak 在 set_self_sync 之后一定存在）
        let self_weak = self
            .self_weak
            .try_read()
            .expect("HomePlugin::rebuild_worker_sync: self_weak lock contended")
            .clone()
            .expect("HomePlugin self_weak not set");

        // 子上下文继承父上下文的环境变量（收口在 `PluginSimpleRequest::child_of`）
        let sub_context = Arc::new(PluginSimpleRequest::child_of(
            &self.context,
            Some(self_weak),
        ));

        // 告知容器它的目录（顶层时就是系统根），以及系统必备插件清单。
        // 容器只知道这个目录；它把它当自己的插件根去扫描——不关心它是不是 homedir。
        sub_context.set(
            PLUGIN_DIR,
            PluginDir::at(HomedirRegistry::get(), PLUGIN_ID_COMPOSITE),
        );
        sub_context.set(
            REQUIRED_PLUGINS,
            SYSTEM_PLUGINS.iter().map(|s| (*s).to_string()).collect(),
        );

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

    /// 切换工作区：候选配置保存成功后才发布到内存。
    pub async fn set_workspace(&self, path_str: &str) -> Result<Value, PluginError> {
        self.set_workspace_in(path_str, &home_dir()).await
    }

    // 显式目录让故障测试不必改动进程级 homedir。
    async fn set_workspace_in(
        &self,
        path_str: &str,
        dir: &PluginDir,
    ) -> Result<Value, PluginError> {
        let expanded_path = shellexpand::tilde(path_str).to_string();

        plugin_info!("home", "正在切换到工作区: {}", expanded_path);

        // 写锁覆盖候选构造、落盘与发布；失败时缓存完全不变。
        let recents = {
            let mut cfg = self.config.write().await;
            let mut candidate = cfg.clone();
            candidate
                .work
                .insert("workdir".to_string(), serde_json::json!(path_str));

            let mut recents = candidate
                .work
                .get("recent_workspaces")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let current_path = serde_json::json!(path_str);
            recents.retain(|v| v != &current_path);
            recents.insert(0, current_path);
            recents.truncate(10);

            candidate.work.insert(
                "recent_workspaces".to_string(),
                Value::Array(recents.clone()),
            );
            dir.save(&candidate)
                .map_err(|e| PluginError::InternalError(format!("持久化自身配置失败: {e}")))?;
            // 保存和发布之间没有 await，避免任务取消留下半次切换。
            *cfg = candidate;

            recents
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<String>>()
        };

        Ok(serde_json::json!({
            "workdir": path_str,
            "expanded_path": expanded_path,
            "recent_workspaces": recents,
            "status": "success"
        }))
    }

    /// 把内存中的配置**原子落盘**到自己的 `PLUGIN.yml`。
    ///
    /// 只写**自己**的配置（工作区 / 最近记录）；每个插件各写自己的文件。
    pub async fn flush(&self) -> Result<(), PluginError> {
        self.flush_in(&home_dir()).await
    }

    async fn flush_in(&self, dir: &PluginDir) -> Result<(), PluginError> {
        // save 使用固定临时文件名；flush 之间也必须串行，不能仅持读锁。
        let cfg = self.config.write().await;
        dir.save(&*cfg)
            .map_err(|e| PluginError::InternalError(format!("持久化自身配置失败: {e}")))?;
        plugin_info!("home", "配置已持久化至: {}", dir.config_path().display());
        Ok(())
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
        Self::new(Arc::new(PluginSimpleRequest::new(None, None)))
    }
}

crate::submit_object_creator!(PLUGIN_ID_HOME, HomePlugin::build, dyn Plugin);

#[async_trait::async_trait]
impl Plugin for HomePlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            // 资源类别清单由 `vdfs/providers`（`<根>` 虚拟根）下发，
            // 导航与顺序都在那里。
            "home/reload" => {
                let req: home_reload::Request = ctx.payload().unwrap_or_default();
                let new_homedir = req.homedir.as_ref().map(PathBuf::from);
                let result = self.reload(new_homedir).await?;

                // 恢复 workdir (需要 Arc<Self> 才能 spawn)
                let workdir_to_restore = {
                    let cfg = self.config.read().await;
                    cfg.work
                        .get("workdir")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                if !workdir_to_restore.is_empty() {
                    let home_arc = Arc::clone(&self);
                    let path_to_restore = workdir_to_restore;
                    tokio::spawn(async move {
                        plugin_info!("home", "reload: 正在恢复 workdir: {}", path_to_restore);
                        if home_arc.set_workspace(&path_to_restore).await.is_ok() {
                            // 同上一处：reload 是唯一会改 workdir 的入口，flush 失败
                            // 意味着「用户改了但没记住」，且此处在后台 spawn 中无声。
                            if let Err(e) = home_arc.flush().await {
                                plugin_warn!("home", "reload 恢复 workdir 后 flush 失败：{}", e);
                            }
                        }
                    });
                }

                return Ok(PluginPayload::new(&result));
            }
            "home/get_homedir" => {
                let info = self.get_homedir_info().await;
                return Ok(PluginPayload::new(&info));
            }
            "work/set_workspace" => {
                #[derive(serde::Deserialize, Clone)]
                struct SetWorkspaceRequest {
                    path: String,
                }
                let req: SetWorkspaceRequest = ctx.payload()?;
                let path_str = &req.path;

                // set_workspace 已确认持久化成功，无需再后台 flush。
                let result = self.set_workspace(path_str).await?;

                return Ok(PluginPayload::new(&result));
            }
            "work/get_workspace" => {
                // 统一从 Home 的配置缓存中读取工作区路径，这是最可靠的数据源
                let cfg = self.config.read().await;
                let work_cfg = &cfg.work;
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
            }
            _ => {}
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
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
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

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
