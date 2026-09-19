//! Composite 插件实现——结构体 + Plugin trait impl
//!
//! 工厂逻辑通过 `Composite::build` 静态方法 + `submit_object_creator!` 自注册。
//!
//! ## 子项从哪来：**插件目录**
//!
//! 容器的子插件由**扫描插件目录**得到（见 [`Composite::mount_from_plugin_dirs`]），
//! 而不是由父插件塞一张配置表进来。于是：
//!
//! - 「有哪些插件」= 插件根下有哪些目录，用户放一个目录就多一个插件；
//! - 「插件怎么配」= 那个目录里的 `PLUGIN.yml`，**插件自己读写**；
//! - 容器只做两件事：把构造者声明的必需插件目录补出来、把合格目录构造出来并把
//!   **自身目录**告知被构造的插件。
//!
//! 父插件因此**不认识任何子插件的配置**——不再有合并规则、不再有分发规则。
//! 容器也**不内置任何插件清单**：它是通用容器（可嵌套另一个容器），「哪些插件
//! 必须存在」由构造者经 [`REQUIRED_PLUGINS`] 随构造传入。
//!
//! ## 容器自己的目录：系统根
//!
//! 容器是 `home` 的**动态内置替身**，父插件把系统根（`<homedir>`）告知它；它据此
//! 定位自己管辖的插件根 `<系统根>/plugins`，因此不依赖任何全局路径常量。
//! 系统根下的 `PLUGIN.yml` 属于 `home`——容器没有配置，不写 manifest。

use super::vdfs::CompositeVdfs;
use crate::symbio_core::{
    create_object, has_creator, lock_read, plugins_root, InvokeRequest, InvokeRequestExt,
    InvokeResponse, Plugin, PluginDir, PluginError, PluginMeta, PluginPayload, SimpleRequest,
    VdfsProvider, CAPABILITY_VISITOR, KEY_PROVIDER, PATH, PLUGIN_COMPOSITE, PLUGIN_DIR,
    PLUGIN_FILE, REQUIRED_PLUGINS, TRAVERSE_AVAILABLE_TOOLS,
};

use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

pub struct Composite {
    instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
    /// 环境变量（层层透传）
    envs: HashMap<String, String>,
    /// VDFS 组合视图：容器是虚拟根 `/` 的拥有者（见 `vdfs` 子模块）
    vdfs: Arc<CompositeVdfs>,
}

impl Composite {
    pub fn new() -> Self {
        Self::new_with_envs(HashMap::new())
    }

    /// 容器**不持有**自己的父引用：它的 `route` 只做子插件分发，从不向上转发
    /// （配置不再经父插件落盘，容器也就没有「向上」的事可做）。子插件的父引用
    /// 由 [`Self::mount_child`] 在子上下文里给，指向本容器。
    pub fn new_with_envs(envs: HashMap<String, String>) -> Self {
        let instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        Self {
            vdfs: Arc::new(CompositeVdfs::new(Arc::clone(&instances))),
            instances,
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
        Self::new()
    }
}

impl Clone for Composite {
    fn clone(&self) -> Self {
        Self {
            instances: Arc::clone(&self.instances),
            envs: self.envs.clone(),
            vdfs: Arc::clone(&self.vdfs),
        }
    }
}

impl Composite {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let envs = if let Some(std_ctx) = ctx.as_any().downcast_ref::<SimpleRequest>() {
            lock_read(&std_ctx.envs).clone()
        } else {
            HashMap::new()
        };

        let composite = Arc::new(Self::new_with_envs(envs));
        let composite_weak = Arc::downgrade(&composite) as Weak<dyn Plugin>;

        Self::mount_from_plugin_dirs(&composite, &ctx, &composite_weak);

        composite as Arc<dyn Plugin>
    }

    /// 装配：**插件目录是子项的唯一来源**
    ///
    /// 1. **构造者声明的必需插件**（[`REQUIRED_PLUGINS`]，随构造经 ctx 传入）：目录 /
    ///    `PLUGIN.yml` 缺失则补出（只补身份字段，缺省的配置字段由插件自己的
    ///    `Default` 兜底）。容器**不内置**任何清单——它是通用容器，甚至可以嵌套
    ///    另一个容器，因此「哪些插件必须存在」是构造者的策略；
    /// 2. 扫描 `<插件根>/*/PLUGIN.yml`：配置**合格**的才加载——文件可解析、且
    ///    `plugin_provider` 指向一个已注册的工厂（判据见 `plugin_dir` 模块文档）；
    /// 3. 构造时把**插件自身目录**经 [`PLUGIN_DIR`] 告知它——它据此自己读写
    ///    `PLUGIN.yml`，容器不碰它的配置。
    ///
    /// 插件根**由容器自己的目录推出**：系统级插件（`home` 及本容器）的目录就是
    /// 系统根，插件根与之**重合**——即插件直接并列在系统根下。父插件把系统根告知
    /// 它，它据此定位自己管辖的地盘，而不是去读一个全局常量。
    /// `home` 的目录是系统根本身而非其下的一层，因此扫描不会构造出第二个 home。
    fn mount_from_plugin_dirs(
        composite: &Arc<Self>,
        ctx: &Arc<dyn InvokeRequest>,
        composite_weak: &Weak<dyn Plugin>,
    ) {
        let root = Self::plugins_root_of(ctx);

        // 1. 构造者声明的必需插件：即便从未配置过也要有一个可编辑的 PLUGIN.yml
        for name in ctx.get(REQUIRED_PLUGINS).unwrap_or_default() {
            if let Err(e) = PluginDir::at(root.join(&name), &name).ensure_manifest() {
                crate::plugin_warn!("composite", "必需插件的配置补建失败 {name}：{e}");
            }
        }

        // 2. 扫描插件根下的一层目录（目录名 = 实例名）
        let mut names: Vec<String> = Vec::new();
        match std::fs::read_dir(&root) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::plugin_warn!("composite", "插件根不存在：{}", root.display());
            }
            Err(e) => crate::plugin_warn!("composite", "读取插件根失败 {}：{e}", root.display()),
        }
        names.sort();

        for name in names {
            let dir = PluginDir::at(root.join(&name), &name);
            let provider = match Self::provider_of(&dir) {
                // 插件根 = 系统根本身，其下**本来就有非插件目录**（如用户自建的
                // 目录）。没有 `PLUGIN.yml` 只是「它不是插件」，不是异常，故只记 debug。
                Ok(None) => {
                    crate::plugin_debug!("composite", "跳过目录（没有 {PLUGIN_FILE}）{name}");
                    continue;
                }
                Ok(Some(p)) => p,
                Err(e) => {
                    crate::plugin_warn!("composite", "跳过插件目录（配置不符合要求）{name}：{e}");
                    continue;
                }
            };
            if !has_creator(&provider) {
                crate::plugin_warn!(
                    "composite",
                    "跳过插件目录（未找到 Provider）{name} -> {provider}"
                );
                continue;
            }
            Self::mount_child(
                composite,
                ctx,
                composite_weak,
                &name,
                &provider,
                dir.with_provider(&provider),
            );
        }
    }

    /// 容器管辖的插件根 = **自己的目录（系统根）本身**
    ///
    /// 父插件经 [`PLUGIN_DIR`] 告知系统根；缺省（测试 / 未装配）退回全局定义
    /// [`plugins_root`]——两者在装配态下是同一个路径。
    fn plugins_root_of(ctx: &Arc<dyn InvokeRequest>) -> PathBuf {
        ctx.get(PLUGIN_DIR)
            .map(|d| d.as_plugins_root())
            .unwrap_or_else(plugins_root)
    }

    /// 插件目录的**身份**：`PLUGIN.yml` 里的 `plugin_provider`
    ///
    /// 三态返回（**刻意区分「不是插件」与「是个坏插件」**）：
    ///
    /// - `Ok(None)`：目录下没有 `PLUGIN.yml` ⇒ 它压根不是插件候选（静默跳过）；
    /// - `Ok(Some(p))`：可加载，`p` 是工厂 id；
    /// - `Err(_)`：有 `PLUGIN.yml` 但不可解析 / 未声明 provider ⇒ **告警**，
    ///   因为「配了一半」是用户需要知道的事。
    fn provider_of(dir: &PluginDir) -> Result<Option<String>, String> {
        let Some(manifest) = dir.read_manifest()? else {
            return Ok(None);
        };
        match manifest.get(KEY_PROVIDER) {
            Some(Value::String(p)) if !p.is_empty() => Ok(Some(p.clone())),
            _ => Err(format!("{PLUGIN_FILE} 未声明 {KEY_PROVIDER}")),
        }
    }

    /// 构造并挂载一个子插件，把它的目录告知它
    fn mount_child(
        composite: &Arc<Self>,
        ctx: &Arc<dyn InvokeRequest>,
        composite_weak: &Weak<dyn Plugin>,
        name: &str,
        provider: &str,
        dir: PluginDir,
    ) {
        // 子上下文只带两样东西：父引用与**自身目录**。
        // 配置不再经 ctx 传递——插件从自己的目录里读。
        let sub_context = Arc::new(SimpleRequest::child_of(ctx, Some(composite_weak.clone())));
        sub_context.set(PLUGIN_DIR, dir);

        crate::plugin_info!("composite", "正在构造子插件 {name} -> {provider}");
        if let Some(plugin_instance) = create_object::<dyn Plugin>(provider, sub_context) {
            composite.add_instance_sync(name.to_string(), plugin_instance);
        } else {
            crate::plugin_warn!("composite", "子插件 Provider 构造失败 {name} -> {provider}");
        }
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

        // 子插件分发
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
        // 装配安排：容器把自己的 vdfs 视图登记进访问层的单槽位，使其成为
        // `.vdfs` 的服务者。这不是「容器是根」——composite 只是恰好包含若干
        // 子目录的 provider，能否出现在那里取决于装配，不是本模块的属性。
        if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                let root: Arc<dyn VdfsProvider> = self.vdfs.clone();
                visitor.register_vdfs_root(root).await;
            }
        }

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
