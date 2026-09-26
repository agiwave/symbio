//! 插件核心 Trait（上下文注入版）

mod dir;
mod error;
mod ids;
mod route;
mod transport;
mod traverse;

// 域内子模块私有，公开面在此显式重导出
pub use dir::{
    plugin_dir_from_ctx, plugin_expand_tilde_path, PluginConfigFile, PluginDir, PluginEntry,
    PluginIdentity, PLUGIN_FILE, PLUGIN_KEY_API, PLUGIN_KEY_AUTHOR, PLUGIN_KEY_CAN_DISABLE,
    PLUGIN_KEY_DESCRIPTION, PLUGIN_KEY_ENABLED, PLUGIN_KEY_GRANTS, PLUGIN_KEY_NAME,
    PLUGIN_KEY_PROVIDER, PLUGIN_KEY_REQUIRED, PLUGIN_KEY_TITLE, PLUGIN_KEY_VERSION,
    PLUGIN_RESERVED_KEYS,
};
pub use error::{PluginError, PluginErrorCode, PluginInvokeResponse};
// 锁辅助函数刻意 `pub(crate)`（见 `error.rs::lock_read` 的说明），不进对外 API
pub(crate) use error::{lock_read, lock_write};
pub use ids::{
    PLUGIN_ID_AGENT, PLUGIN_ID_COMPOSITE, PLUGIN_ID_EVENT_BUS, PLUGIN_ID_GATEWAY, PLUGIN_ID_HOME,
    PLUGIN_ID_HOOK, PLUGIN_ID_LOCAL, PLUGIN_ID_MANAGER, PLUGIN_ID_MCP, PLUGIN_ID_MODEL,
    PLUGIN_ID_SESSION, PLUGIN_ID_SKILL, PLUGIN_ID_TELEGRAM, PLUGIN_ID_VDFS, PLUGIN_ID_WEB,
    PLUGIN_ID_WORK,
};
pub use route::{
    ROUTE_EVENT_BUS_SUBSCRIBE, ROUTE_HOOK_FIRE, ROUTE_SESSION_CHAT_ABORT, ROUTE_SESSION_CHAT_SEND,
    ROUTE_VDFS_ROOT, ROUTE_VDFS_UNWATCH, ROUTE_VDFS_WATCH,
};
pub use transport::{
    PluginChannel, PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire,
};
pub use traverse::{TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS};

use crate::symbio_core::SymbioKey;
use crate::symbio_core::{
    VdfsAccess, VdfsContext, VdfsError, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock, Weak};

/// 插件的**出厂自述**（ADR-032）—— 两半，消费方式**不同**
///
/// | 半 | 字段 | 消费方式 |
/// |---|---|---|
/// | **身份** | `id` / `name` / `description` / `version` / `author` | 装配期**一次性投影**进 `PLUGIN.yml`（[`PluginDir::seed_identity`](crate::symbio_core::PluginDir::seed_identity)），此后运行期读 manifest |
/// | **挂载点呈现** | `order` / `icon` / `hidden` / `root_access` | 运行期读：`composite/vdfs.rs` 合成挂载点目录节点、`composite/registry.rs` 排序 |
///
/// 为什么分两半：**身份必须常在**（「这个插件叫什么」与它开没开无关，停用的插件
/// 也要能在列表里显示名字），而**挂载点呈现只在挂载时成立**（没有挂载点，
/// `order` / `hidden` 无从谈起）。前者因此落进 `PLUGIN.yml`（跟着目录走），
/// 后者留在本结构（跟着构造物走）。
///
/// `name` 是**挂载点标题**（同时是插件展示名）：它投影成 manifest 的 `plugin_title`，
/// 运行期的目录节点标题取自那里。
///
/// 无 VDFS 挂载的插件，挂载点呈现那半保持缺省，无副作用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub id: String,
    /// 挂载点标题（VDFS 目录节点的 `title`；同时是插件展示名）
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    /// 导航排序（小者靠前；缺省 100）
    #[serde(default = "default_meta_order")]
    pub order: i32,
    /// 图标名（使用方纯 UI 映射；缺省无）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 挂载点在父目录列表中的隐藏位（缺省不隐藏；语义与 [`VdfsNode::hidden`](crate::symbio_core::VdfsNode::hidden) 一致）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// 挂载点（根目录）的访问位（缺省「可列目录」）
    #[serde(default = "default_meta_root_access")]
    pub root_access: VdfsAccess,
    // 注：挂载点根下「可新建类型」（`VdfsNode::new_type`）**有意不在这里**——
    // session 的表单 schema 需要运行期汇流（options 广播），而本结构是同步纯数据。
    // 它挂在**根节点自己的自述**上，由容器向 provider 发一次 `Stat` 现场取
    // （见 `docs/DECISIONS.md` ADR-030）。
}

fn default_meta_order() -> i32 {
    100
}

fn default_meta_root_access() -> VdfsAccess {
    VdfsAccess::LIST
}

impl Default for PluginMeta {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: None,
            version: None,
            author: None,
            order: default_meta_order(),
            icon: None,
            hidden: false,
            root_access: default_meta_root_access(),
        }
    }
}

impl PluginMeta {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// 导航排序（小者靠前）
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// 图标名（使用方纯 UI 映射）
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// 挂载点隐藏位（在父目录列表中不显示；可达性不受影响）
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// 挂载点（根目录）访问位
    pub fn with_root_access(mut self, access: VdfsAccess) -> Self {
        self.root_access = access;
        self
    }
}

// PluginInvokeRequest Trait & PluginSimpleRequest

/// 载荷键名 —— 上下文里那个**由调用方定类型**的桶
///
/// `PluginInvokeRequestExt::payload` / `set_payload` 读写的就是它。它**不是一个
/// [`SymbioKey`]**：值类型由调用方决定（`payload::<T>()` 的 `T`），而 `SymbioKey`
/// 要求一个固定的关联 `Value` 类型——故这里是纯字符串常量，不是键实例。
///
/// 归属本域：它描述的是信封里那个载荷桶，与 [`PluginPayload`] 是同一概念面；
/// `keys` 域只收 `SymbioKey` 的实例与类型。
///
/// 只有一个定义处，读写双方都引它——`grep-audit` 的 S-009 拦的正是「绕开常量写裸
/// 字符串 `"payload"`」那种形态。
pub const PLUGIN_PAYLOAD_KEY: &str = "payload";

/// 插件上下文接口 - Symbio 架构的“血液”
///
/// 采用类型擦除模式，支持跨层级的能力注入与透传
pub trait PluginInvokeRequest: Send + Sync {
    /// 内部原始提取器
    fn get_raw(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>>;

    /// 内部原始设置器，用于支持上下文变异
    fn set_raw(&self, key: &str, value: Arc<dyn Any + Send + Sync>);

    /// 克隆一个独立的上下文副本 (用于转发请求而不影响当前链路)
    fn fork(&self) -> Arc<dyn PluginInvokeRequest>;

    /// 支持向下转型 (用于在分形层级中获取具体实现)
    fn as_any(&self) -> &dyn Any;
}

/// 插件上下文扩展 - 为开发者提供优雅的类型安全 API
pub trait PluginInvokeRequestExt: PluginInvokeRequest {
    /// 获取特定键的值或接口
    fn get<K: SymbioKey>(&self, key: K) -> Option<K::Value> {
        self.get_raw(key.name())
            .and_then(|any| any.downcast::<K::Value>().ok())
            .map(|arc| (*arc).clone())
    }

    /// 设置特定键的值或接口
    fn set<K: SymbioKey>(&self, key: K, value: K::Value) {
        self.set_raw(key.name(), Arc::new(value));
    }

    /// 获取父插件引用 (Weak)
    fn parent(&self) -> Option<std::sync::Weak<dyn crate::symbio_core::Plugin>> {
        self.get(crate::symbio_core::PARENT).flatten()
    }

    /// 获取配置信息 (Value)
    fn config(&self) -> Option<serde_json::Value> {
        self.get(crate::symbio_core::CONFIG)
    }

    /// 直接将上下文中的载荷解析为指定的强类型 T（进程内零拷贝）
    ///
    /// 优先尝试原生类型转换，失败时自动回退到 JSON 反序列化。
    /// 读写的桶名见 [`crate::symbio_core::PLUGIN_PAYLOAD_KEY`]。
    fn payload<T: serde::de::DeserializeOwned + Clone + Send + Sync + 'static>(
        &self,
    ) -> Result<T, crate::symbio_core::PluginError> {
        let key = crate::symbio_core::PLUGIN_PAYLOAD_KEY;
        let any = self.get_raw(key).ok_or_else(|| {
            crate::symbio_core::PluginError::ValidationError(
                "Missing payload in context".to_string(),
            )
        })?;

        // 优先尝试直接类型转换（零拷贝）
        if let Ok(val) = any.clone().downcast::<T>() {
            return Ok((*val).clone());
        }

        // 尝试作为 JSON Value 反序列化
        if let Ok(json_val) = any.downcast::<serde_json::Value>() {
            return serde_json::from_value((*json_val).clone()).map_err(|e| {
                crate::symbio_core::PluginError::ValidationError(format!(
                    "参数反序列化失败，请核对契约类型或 Schema: {e}"
                ))
            });
        }

        Err(crate::symbio_core::PluginError::ValidationError(
            "Payload type mismatch".to_string(),
        ))
    }

    /// 设置上下文中的载荷数据（进程内零拷贝）
    ///
    /// 只保留**进程内零拷贝**这一种写入口（`Arc` 直存扩展桶），刻意不再提供
    /// "先 `serde_json::to_value` 再存"的旧版本：写入侧强加 `Serialize` 会挡掉那些
    /// 本就不该被序列化的进程内对象，并让每次转发白付一次序列化开销。序列化路径
    /// 放在读取侧——[`PluginInvokeRequestExt::payload`] 自带"原生 downcast → JSON 反序列化"
    /// 的两段回退，需要它的地方才付这个代价。
    fn set_payload<T: Clone + Send + Sync + 'static>(
        &self,
        value: T,
    ) -> Result<(), crate::symbio_core::PluginError> {
        let key = crate::symbio_core::PLUGIN_PAYLOAD_KEY;
        self.set_raw(key, Arc::new(value));
        Ok(())
    }
}

impl<T: PluginInvokeRequest + ?Sized> PluginInvokeRequestExt for T {}

/// 标准上下文实现
pub struct PluginSimpleRequest {
    /// 环境变量桶
    pub envs: Arc<RwLock<HashMap<String, String>>>,
    /// 万能扩展桶 (用于进程内透传复杂 Rust 对象，如 Payload, Metadata, PARENT, CONFIG 等)
    pub extensions: Arc<RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl PluginSimpleRequest {
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: Option<Value>) -> Self {
        let mut extensions = HashMap::new();
        if let Some(p) = parent {
            extensions.insert(
                crate::symbio_core::PARENT.name().to_string(),
                Arc::new(Some(p)) as Arc<dyn Any + Send + Sync>,
            );
        }
        if let Some(c) = config {
            extensions.insert(
                crate::symbio_core::CONFIG.name().to_string(),
                Arc::new(c) as Arc<dyn Any + Send + Sync>,
            );
        }
        Self {
            envs: Arc::new(RwLock::new(HashMap::new())),
            extensions: Arc::new(RwLock::new(extensions)),
        }
    }

    /// 派生一个子上下文：`parent` 是子上下文的父引用，环境变量从 `from` 快照一份。
    ///
    /// 收口三处装配点的重复写法（`composite` 挂子插件、`home` 造 worker、`agent`
    /// 造子 Agent）。注意这两个参数**常常不是同一个对象**：容器挂子插件时父引用是
    /// 容器自己，而环境变量要顺着**外层请求上下文**继续往下传。
    ///
    /// 子上下文拿到的是环境的**副本**而非共享句柄——各子插件此后互不影响。
    /// `from` 若并非标准上下文实现，则无环境可继承（子上下文 envs 为空）。
    pub fn child_of(from: &Arc<dyn PluginInvokeRequest>, parent: Option<Weak<dyn Plugin>>) -> Self {
        let child = Self::new(parent, None);
        if let Some(src) = from.as_any().downcast_ref::<PluginSimpleRequest>() {
            *lock_write(&child.envs) = lock_read(&src.envs).clone();
        }
        child
    }
}

impl PluginInvokeRequest for PluginSimpleRequest {
    fn get_raw(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        // 1. 优先从扩展桶获取 (包括存储在此的 Payload、Metadata 等所有临时/高级对象)
        if let Ok(exts) = self.extensions.read() {
            if let Some(val) = exts.get(key) {
                return Some(Arc::clone(val));
            }
        }

        // 2. 兜底从环境变量获取 (Envs)
        if let Ok(envs) = self.envs.read() {
            if let Some(val) = envs.get(key) {
                return Some(Arc::new(val.clone()));
            }
        }

        None
    }

    fn set_raw(&self, key: &str, value: Arc<dyn Any + Send + Sync>) {
        if let Ok(mut exts) = self.extensions.write() {
            exts.insert(key.to_string(), value);
        }
    }

    fn fork(&self) -> Arc<dyn PluginInvokeRequest> {
        Arc::new(Self {
            envs: Arc::clone(&self.envs),
            extensions: Arc::new(RwLock::new(lock_read(&self.extensions).clone())),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// 插件被停下的**原因**（ADR-033）
///
/// 三态而不是一个布尔量：插件的**处置不同**——「卸载时是否保留自己的缓存 / 数据」
/// 是可恢复停用时不必做、卸载时必须当场决定的事。塞进布尔量，就会在插件里长出一堆
/// 「我这次到底是为什么被停」的旁门判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStopReason {
    /// 可恢复的停用——目录与数据**都在**，随时可以再启用
    Disabled,
    /// 卸载——调用方随后会删掉插件目录
    Uninstalled,
    /// 全局收尾（进程退出 / 整棵树被丢弃）
    Shutdown,
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 插件的**出厂自述** —— 身份（种子）+ 挂载点呈现（见 ADR-032）
    ///
    /// 两半的消费方式不同，别混：
    ///
    /// - **身份**（`name` / `description` / `version` / `author`）：只在**装配期**
    ///   被读一次——容器构造出插件后经 `PluginDir::seed_identity` 投影进
    ///   `PLUGIN.yml`（键缺失才写）。此后 manifest 是权威，运行期一律读
    ///   `PluginDir::identity()`，因此**停用的插件也有身份**。
    /// - **挂载点呈现**（`order` / `icon` / `hidden` / `root_access`）：运行期读，
    ///   消费方是 `composite/vdfs.rs`（合成挂载点目录节点）与 `composite/registry.rs`
    ///   （`order` 用于排序）。它们描述「这个挂载点长什么样」，没有挂载点就无从谈起，
    ///   所以**不投影**。
    ///
    /// 其余 `.meta()` 调用点命中的都是 `Capability::meta`（能力，而非插件）。
    ///
    /// 之所以仍留在 trait 上、且身份字段**尚未删除**，是**刻意的**：删除它们需要
    /// 「**不构造也能拿到出厂身份**」的能力——一张按工厂 id 索引的静态自述注册表，
    /// 属第二期（provider 两级解析）的产物。本期先把**运行期消费路径**单源化。
    /// 若将来确认要删，请连同各 impl 一并清理，不要只删这一行声明。
    fn meta(&self) -> PluginMeta;

    /// 装配后、开始服务前调用一次（**同步**，见 ADR-033）
    ///
    /// 默认无操作——16 个内置插件因此一行不改、行为与加钩子前完全一致。
    ///
    /// 为什么是**同步**的：装配路径（`HomePlugin::build` → `Composite::build` →
    /// `PluginRegistry::mount_all`）是同步的，工厂签名本身非 async，在那里 await
    /// 不可用。需要异步初始化的插件（外部插件）走另外两步：**构造时同步 spawn
    /// 子进程** + **首次调用前惰性完成 `init`**。
    fn start(&self, _ctx: Arc<dyn PluginInvokeRequest>) -> Result<(), PluginError> {
        Ok(())
    }

    /// 停用 / 卸载 / 收尾时调用（**异步**，见 ADR-033）
    ///
    /// 默认无操作。调用点一律在**移除实例之前**（停用）或**删除目录之前**（卸载）——
    /// 顺序反了就等于让插件在数据已经没了之后再去做清理；卸载时尤其致命，
    /// 这是插件**最后**一次能写盘的机会。
    ///
    /// 为什么是**异步**的：全部调用点都已在 async 上下文，而清理本身需要 await
    /// （发 shutdown 帧、等子进程退出、flush 落盘）——同步的 `Drop` 做不了这些。
    async fn stop(self: Arc<Self>, _reason: PluginStopReason) -> Result<(), PluginError> {
        Ok(())
    }

    /// 分形路由入口：接收一个抽象的上下文对象，按需提取参数
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload>;

    /// 分形遍历接口
    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload>;

    /// 取本插件直接暴露的虚拟文件系统 provider（**系统链路 / 文件系统视角**）。
    ///
    /// 与 `CapabilityVisitor`（LLM 链路的能力收集，含 `get_vdfs_root`）**无关**：
    /// 这里是「这个插件自己暴露的 VDFS 视图」的直接查询接口——容器经它把子插件的
    /// provider 聚合进组合视图，子智能体经它穿过 `agent/<id>` 挂载点。两条链路拿到的
    /// 是同一个 provider 实例，只是发现通道不同。
    ///
    /// 默认 `None`：大多数插件不暴露 VDFS。容器（`Composite`）返回自己的
    /// `CompositeVdfs`；自身即 provider 的插件（session / model / mcp / skill /
    /// plugin_manager / agent / …）返回 `self`（或插件内聚的 provider 句柄）。
    ///
    /// ⚠️ 这是 core 查询接口（`Plugin` trait），不引入任何插件间类型耦合——
    /// 调用方只依赖 `Arc<dyn Plugin>`，绝不依赖某个具体插件类型。
    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        None
    }

    /// 把一次 VDFS 请求派发进本插件（**LLM 工具 / 协议层调用的唯一入口**）。
    ///
    /// 默认实现经 [`Self::get_vfs_provider`] 转发：自身即 provider 的插件什么都不
    /// 用做；不暴露 VDFS 的插件得到 [`VdfsError::NotImplemented`]。拆成独立方法的
    /// 理由：
    ///
    /// 1. **挂载名与目录名天然同一份**——容器、agent 作用域代理等「按目录名转发」
    ///    的派发方不再需要为每个子插件先调 `get_vfs_provider` 再调 provider，
    ///    一跳直达；
    /// 2. 需要把 VDFS 实现拆进**独立 provider 结构**（高内聚、多文件）的插件
    ///    （如 agent / session 的复合资源域）可同时实现两者，派发面与实现面分离；
    /// 3. 转发型的「目录名 → 相对路径 + 子插件派发」不需要实例化任何中间结构。
    async fn vdfs_dispatch(
        self: Arc<Self>,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match self.get_vfs_provider() {
            Some(p) => p.dispatch(ctx, path, req).await,
            None => Err(VdfsError::NotImplemented),
        }
    }
}

#[cfg(test)]
mod tests;
