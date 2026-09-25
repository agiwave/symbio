//! 插件注册表 —— 容器管辖的**插件目录集合**，以及它的装配与运行期增删
//!
//! ## 它是什么
//!
//! 「这个智能体由哪些插件组成」是一个**事实**，事实的所在地是磁盘：插件根下的
//! 一层目录 = 一个插件（见 `symbio_core::plugin::dir` 的模块文档）。本模块把那层
//! 目录 + 实例表收成一个对象，于是：
//!
//! | 谁 | 干什么 |
//! |---|---|
//! | 容器（[`super::Composite`]） | 持有本表；`route` / `traverse` 经它取实例 |
//! | 容器的 VDFS（[`super::CompositeVdfs`]） | 持有本表；根上的注册表动词经它执行 |
//! | 插件管理插件（`plugins::plugin_manager`） | **只读**它产出的 [`PluginEntry`]，不自己扫目录 |
//!
//! 三条判据因此只有一份实现（原先散在 `Composite::mount_from_plugin_dirs` 里，
//! 只有装配期那一次调用）：
//!
//! - **合格性**：[`PluginRegistry::provider_of`] 的三态（不是插件 / 可加载 / 配了一半）；
//! - **必需**：构造者经 [`REQUIRED_PLUGINS`] 声明的清单（缺目录就补出来）；
//! - **启用**：[`KEY_ENABLED`]（`PLUGIN.yml` 里的装配位，缺省 = 启用）。
//!
//! ## 为什么运行期能改，而不用重启
//!
//! 实例表是**唯一**的「已挂载插件」来源（`route` / `traverse` / 资源树都现取它），
//! 因此启停与增删只要**改这张表 + 改磁盘上的装配位**，树与能力集合立刻跟着变——
//! 不需要 `home/reload`，也不需要在别处同步第二份状态。
//!
//! ## 一个刻意的边界：停用的插件**不被构造**
//!
//! 停用不是「构造了但不显示」，而是**根本不构造**：它不启动任何后台行为
//! （监听、定时任务、连接池），这正是用户按下「停用」时想要的东西。
//!
//! 这曾经带来一个代价：`PluginMeta` 是构造物，停用就拿不到标题 / 版本 / 描述，
//! [`PluginEntry`] 的那几个字段因此为空、消费方按目录名兜底。**现已解决**：身份归
//! `PLUGIN.yml`（ADR-032），装配期由 [`PluginRegistry::mount_child`] 把出厂身份
//! **投影进 manifest 一次**，此后运行期只读 manifest——停用的插件照样有名字。
//!
//! 挂载点呈现（`order` / `hidden` / `root_access`）仍取自 `PluginMeta`，这是刻意的：
//! 没有挂载点就没有这些属性，停用插件的 `order` 取缺省正是「它现在不在树里」的表达。
//!
//! 顺带一个后果：停用期间它的 `PLUGIN.yml` 不再经 VDFS 可达（提供者没被构造），
//! 配置要等重新启用后才能改。这是「停用」的题中之义——**停用是把这个插件连同
//! 它的配置一起停下**，而不是把它留在树里只关掉行为。

use crate::symbio_core::{
    create_object, creator_ids, has_creator, lock_read, lock_write, Plugin, PluginDir, PluginEntry,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginMeta, PluginSimpleRequest, PluginStopReason,
    KEY_PROVIDER, PLUGIN_DIR, PLUGIN_FILE, REQUIRED_PLUGINS, SYSTEM_LEVEL_PROVIDERS,
    UNDISABLABLE_PLUGINS,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Weak};

/// 容器管辖的插件集合（见模块文档）
pub struct PluginRegistry {
    /// 已挂载实例：目录名 → 插件。**运行期唯一**的「谁在装配中」来源。
    ///
    /// 用 `std::sync::RwLock` 而非 tokio 版：临界区只有「取一张表的快照」或
    /// 「插一项」，没有任何 await；而装配期（`build`）是**同步**上下文，拿不到
    /// `.await`。同一个锁两种上下文都能用，才不必把装配拆成两份实现。
    instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
    /// 插件根（= 本容器的目录；系统级容器的目录就是系统根）
    root: PathBuf,
    /// 构造者声明的必需插件（目录必须存在；**不可删除**，但可停用）
    required: Vec<String>,
    /// 构造上下文：运行期挂载新插件时由它派生子上下文
    ctx: Arc<dyn PluginInvokeRequest>,
    /// 容器自身——子插件的父引用（子插件经 `ctx.parent()` 回到容器）。
    /// `None` = 独立构造（测试 / 无容器场景），此时被挂上的子插件没有父可回。
    parent: Option<Weak<dyn Plugin>>,
}

impl PluginRegistry {
    /// 装配期构造：插件根 / 必需清单 / 构造上下文都来自 ctx
    pub fn new(ctx: Arc<dyn PluginInvokeRequest>, parent: Weak<dyn Plugin>) -> Self {
        // 容器只知道**自己的目录**（父插件经 `PLUGIN_DIR` 告知）；顶层时它恰好是
        // homedir，非顶层（子智能体）则不是。**没有全局回退**——容器不读 homedir。
        let root = ctx
            .get(PLUGIN_DIR)
            .map(|d| d.as_plugins_root())
            .expect("composite 需要父插件经 PLUGIN_DIR 告知自己的目录");
        let required = ctx.get(REQUIRED_PLUGINS).unwrap_or_default();
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            root,
            required,
            ctx,
            parent: Some(parent),
        }
    }

    /// 直接给定实例表 / 插件根 / 必需清单（**仅测试**）。
    ///
    /// 与 [`Self::new`] 的差别只有「不扫目录、无父引用」：装配期那一步由调用方
    /// 自己按需触发（`ensure_required` / `mount_all`）。需要自己指定插件根的场景
    /// 只有测试——真实装配的根永远来自 `ctx`。生产路径没有它，所以不进 `pub` 面。
    #[cfg(test)]
    pub fn with_instances(
        instances: Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>>,
        root: PathBuf,
        required: Vec<String>,
    ) -> Self {
        Self {
            instances,
            root,
            required,
            ctx: Arc::new(PluginSimpleRequest::new(None, None)),
            parent: None,
        }
    }

    /// 实例表句柄（`Composite` 需要它与容器共享同一份，见 `composite.rs`）
    pub fn instances(&self) -> &Arc<RwLock<HashMap<String, Arc<dyn Plugin>>>> {
        &self.instances
    }

    /// 已挂载实例的快照（容器 `route` / `traverse` / 资源树共用）
    pub fn snapshot(&self) -> Vec<(String, Arc<dyn Plugin>)> {
        lock_read(&self.instances)
            .iter()
            .map(|(n, p)| (n.clone(), Arc::clone(p)))
            .collect()
    }

    /// 构造者是否把 `name` 声明为必需（= 不可删除）
    pub fn is_required(&self, name: &str) -> bool {
        self.required.iter().any(|n| n == name)
    }

    /// 某插件的目录（**公开**：资源树合成挂载点节点时也要读它的身份，见 ADR-032）
    pub fn dir_of(&self, name: &str) -> PluginDir {
        PluginDir::at(self.root.join(name), name)
    }

    // ==================== 装配期 ====================

    /// 构造者声明的必需插件：即便从未配置过也要有一个可编辑的 `PLUGIN.yml`
    pub fn ensure_required(&self) {
        for name in &self.required {
            if let Err(e) = PluginDir::at(self.root.join(name), name).ensure_manifest() {
                crate::plugin_warn!("composite", "必需插件的配置补建失败 {name}：{e}");
            }
        }
    }

    /// 装配：**插件目录是子项的唯一来源**
    ///
    /// 扫描 `<插件根>/*/`：配置**合格**（有 `PLUGIN.yml` 且 `plugin_provider` 指向
    /// 一个已注册工厂）且**启用**（见 [`KEY_ENABLED`]）的才构造。
    pub fn mount_all(&self) {
        for name in self.dir_names() {
            let dir = self.dir_of(&name);
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
            let dir = dir.with_provider(&provider);
            if !dir.enabled() {
                crate::plugin_debug!("composite", "跳过已停用的插件 {name} -> {provider}");
                continue;
            }
            self.mount_child(&name, &provider, dir);
        }
    }

    /// 插件根下的一层目录名（排序后；目录不存在时为空）
    fn dir_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        match std::fs::read_dir(&self.root) {
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
                crate::plugin_warn!("composite", "插件根不存在：{}", self.root.display());
            }
            Err(e) => {
                crate::plugin_warn!("composite", "读取插件根失败 {}：{e}", self.root.display())
            }
        }
        names.sort();
        names
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
    ///
    /// 子上下文只带两样东西：父引用与**自身目录**。配置不经 ctx 传递——
    /// 插件从自己的目录里读。
    fn mount_child(&self, name: &str, provider: &str, dir: PluginDir) {
        let sub = PluginSimpleRequest::child_of(&self.ctx, self.parent.clone());
        sub.set(PLUGIN_DIR, dir.clone());
        let sub_context: Arc<dyn PluginInvokeRequest> = Arc::new(sub);

        // 装配细节走 debug：每个子插件一行，十几个插件连成一串纯机械噪声，
        // 用户视角「启动刷屏」的主要来源。需要排查装配问题时 `--verbose` /
        // `SYMBIO_LOG=debug` 即可看到全量。
        crate::plugin_debug!("composite", "正在构造子插件 {name} -> {provider}");
        match create_object::<dyn Plugin>(provider, Arc::clone(&sub_context)) {
            Some(plugin) => {
                // **出厂身份投影**（ADR-032）：构造成功后才拿得到 `PluginMeta`，
                // 于是就在这一刻把身份补进 manifest（只补缺失的键，用户改过的不动）。
                // 此后运行期只读 manifest，停用插件因此也有名字——这条投影是
                // 「停用不失身份」的**唯一**来路，别处不必再读 `meta()` 的身份字段。
                if let Err(e) = dir.seed_identity(&plugin.meta()) {
                    crate::plugin_warn!("composite", "插件身份落位失败 {name}：{e}");
                }
                // **生命周期钩子**（ADR-033）：装配后、开始服务前一次。
                // 同步——装配路径里没有 async 上下文（见该 ADR）。
                // 失败**不摘掉**插件：与 `provider_of` 同一口径——配了一半也要看得见，
                // 摘掉它用户只看到「插件不见了」，留着则每次调用给出明确错误。
                if let Err(e) = plugin.start(sub_context) {
                    crate::plugin_error!("composite", "子插件启动失败 {}：{}", name, e);
                }
                lock_write(&self.instances).insert(name.to_string(), plugin);
            }
            None => {
                crate::plugin_warn!("composite", "子插件 Provider 构造失败 {name} -> {provider}")
            }
        }
    }

    // ==================== 注册表（运行期观测） ====================

    /// 注册表全表：插件根下每一个**合格**目录一条（含已停用、含已构造失败的）。
    ///
    /// 「合格」= 有 `PLUGIN.yml` 且声明了非空 `plugin_provider`——**不要求**该工厂
    /// 已注册：配错工厂名的目录也要出现在表里，否则用户既看不到它、也删不掉它。
    ///
    /// 每次现取（不缓存）：目录可能在应用之外被增删，缓存就多一份会漂的真相。
    pub fn entries(&self) -> Vec<PluginEntry> {
        let mounted = lock_read(&self.instances);
        let mut out: Vec<PluginEntry> = Vec::new();
        for name in self.dir_names() {
            let dir = self.dir_of(&name);
            let provider = match Self::provider_of(&dir) {
                Ok(Some(p)) => p,
                // 不是插件候选（无 manifest）/ 配了一半：都不进注册表
                _ => continue,
            };
            let dir = dir.with_provider(&provider);
            // 身份从 **manifest** 读（ADR-032）——停用的插件也有名字；只有从未
            // 落位过的目录才为空，消费方按 `name` 兜底。
            let identity = dir.identity();
            // 排序用的 `order` 是**挂载点呈现**，仍取自构造物——没挂载就没有位置。
            let order = mounted
                .get(&name)
                .map(|p| p.meta().order)
                .unwrap_or_else(|| PluginMeta::default().order);
            out.push(PluginEntry {
                title: identity.title,
                description: identity.description,
                version: identity.version,
                order,
                required: self.is_required(&name),
                enabled: dir.enabled(),
                mounted: mounted.contains_key(&name),
                name: name.clone(),
                provider,
            });
        }
        out.sort_by(|a, b| (a.order, &a.name).cmp(&(b.order, &b.name)));
        out
    }

    /// 可安装的插件工厂：已注册、但**当前容器里没有它的挂载实例**（顺序 = 工厂 id 升序）
    ///
    /// 「未挂载」= 注册表里没有 `mounted` 的那一条。于是**停用**的插件也在候选里——
    /// 这正是用户按下「添加插件」时的语义：把它重新装配进来（已停用的只需启用）。
    /// 系统级工厂（`home` / `composite`）排除在外：它们的目录就是系统根本身。
    pub fn installable(&self) -> Vec<&'static str> {
        let mounted = lock_read(&self.instances);
        let mut ids: Vec<&'static str> = creator_ids::<dyn Plugin>()
            .into_iter()
            .filter(|id| !SYSTEM_LEVEL_PROVIDERS.contains(id))
            .filter(|id| !mounted.contains_key(*id))
            .collect();
        ids.sort_unstable();
        ids
    }

    // ==================== 运行期：启停 / 安装 / 卸载 ====================

    /// 写装配位并即时生效：`true` = 挂载，`false` = 卸载（实例表随之增删）。
    ///
    /// 界面底座插件（[`UNDISABLABLE_PLUGINS`]）拒绝停用——理由见该常量文档。
    ///
    /// 停用一侧会先调 [`Plugin::stop`](Plugin::stop)（ADR-033）：**摘实例之前**，
    /// 让插件有一次清理的机会（关监听 / 停后台任务 / 落盘）。
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        if !enabled && UNDISABLABLE_PLUGINS.contains(&name) {
            return Err(format!(
                "「{name}」是界面底座插件，停用后界面将无法再操作它（如需停用请直接改 {PLUGIN_FILE}）"
            ));
        }
        let dir = self.dir_of(name);
        let provider = match Self::provider_of(&dir)? {
            Some(p) => p,
            None => return Err(format!("未找到插件：{name}")),
        };
        let dir = dir.with_provider(&provider);
        dir.set_enabled(enabled)?;

        if enabled {
            if !lock_read(&self.instances).contains_key(name) {
                if !has_creator(&provider) {
                    return Err(format!("未注册的插件工厂：{provider}"));
                }
                self.mount_child(name, &provider, dir);
                if !lock_read(&self.instances).contains_key(name) {
                    return Err(format!("插件「{name}」构造失败，请查看日志"));
                }
            }
        } else {
            // 先摘出来再 await：写锁守卫不能跨 await（future 会失去 Send）。
            let plugin = lock_write(&self.instances).remove(name);
            if let Some(p) = plugin {
                // ADR-033：摘出实例表**之后立即** stop（Disabled = 可恢复，目录与数据都还在）。
                // 失败只告警、不阻断——用户按下「停用」就该生效，清理没做完不该反过来
                // 让插件停不掉（那时插件只剩 `Drop`，而 `Drop` 不能 await）。
                if let Err(e) = p.stop(PluginStopReason::Disabled).await {
                    crate::plugin_warn!("composite", "插件停用时清理失败 {name}：{e}");
                }
            }
        }
        Ok(())
    }

    /// 安装：把一个**已注册的插件工厂**装进本容器。
    ///
    /// 幂等，且对两种「未挂载」是同一个动作——让这个工厂进入装配：
    ///
    /// - 已有插件目录（此前被停用）⇒ 写回启用位并挂载；
    /// - 没有插件目录（从未装过）⇒ 建出目录（只写身份字段）再挂载。
    ///
    /// 名字 = 工厂 id：目录名 = 挂载名 = 实例名（见 `symbio_core::plugin::dir`）。
    /// 返回新挂上的插件名。
    pub fn install(&self, provider: &str) -> Result<String, String> {
        let provider = provider.trim();
        if provider.is_empty() {
            return Err("未指定插件工厂".to_string());
        }
        if SYSTEM_LEVEL_PROVIDERS.contains(&provider) {
            return Err(format!("「{provider}」是系统级插件，不可作为子插件装配"));
        }
        if !has_creator(provider) {
            return Err(format!("未注册的插件工厂：{provider}"));
        }
        if lock_read(&self.instances).contains_key(provider) {
            return Err(format!("插件「{provider}」已在装配中"));
        }
        let dir = self.dir_of(provider);
        dir.ensure_manifest()
            .map_err(|e| format!("创建插件目录失败：{e}"))?;
        dir.set_enabled(true)?;
        self.mount_child(provider, provider, dir.with_provider(provider));
        if !lock_read(&self.instances).contains_key(provider) {
            return Err(format!("插件「{provider}」构造失败，请查看日志"));
        }
        Ok(provider.to_string())
    }

    /// 卸载：从装配中摘掉并**删掉它的插件目录**（连同目录里的配置与数据）。
    ///
    /// 只对**非必需**插件可用——必需插件是构造者声明的「这个智能体必须有它」，
    /// 用户能做的是停用（见 [`Self::set_enabled`]），不是删除。
    ///
    /// 顺序（ADR-033）：`stop(Uninstalled)` 在**删目录之前**——那是插件**最后**
    /// 一次能写盘的机会。
    pub async fn uninstall(&self, name: &str) -> Result<(), String> {
        if self.is_required(name) {
            return Err(format!("「{name}」是必需插件，不可删除（可以停用）"));
        }
        let dir = self.dir_of(name);
        if !dir.config_path().exists() && !lock_read(&self.instances).contains_key(name) {
            return Err(format!("未找到插件：{name}"));
        }
        // 与停用同一口径：清理失败只告警，删除继续——目录随后就没了，
        // 再卡在这里只会留下一个「卸不掉」的僵尸。
        // 先把实例摘出来再 await：写锁守卫不能跨 await（future 会失去 Send）。
        let plugin = lock_write(&self.instances).remove(name);
        if let Some(p) = plugin {
            if let Err(e) = p.stop(PluginStopReason::Uninstalled).await {
                crate::plugin_warn!("composite", "插件卸载时清理失败 {name}：{e}");
            }
        }
        std::fs::remove_dir_all(dir.dir())
            .map_err(|e| format!("删除插件目录 {} 失败：{e}", dir.dir().display()))
    }
}

#[cfg(test)]
#[path = "registry.test.rs"]
mod tests;
