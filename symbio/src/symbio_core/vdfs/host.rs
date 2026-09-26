//! VDFS 宿主桥（symbio 侧）—— 把纯接口接到 symbio 类型上
//!
//! ⚠️ 这是 core 中**唯一**依赖宿主的部分。它只做三件事：
//!
//! 1. **上下文注入**：`Arc<dyn PluginInvokeRequest>` ↔ [`VdfsContext`]
//!    （provider 经 `ctx.require::<Arc<dyn PluginInvokeRequest>>()` 取回宿主句柄）；
//! 2. **错误翻译**：[`VdfsError`] ↔ [`PluginError`] 双向映射；
//! 3. **变更广播**：挂载点写 / 删后 [`notify_change`]，`watch` 经
//!    [`watch_changes`] 订阅后转发——前端因此无需轮询（**非**轮询实现）。
//!    转发由 [`VdfsChangeSubscriptions`] 统一收敛：**一条变更只会出总线一次**
//!    （命中多条相关订阅时也只调用一个投递器），同一路径的多位订阅者
//!    按引用计数配对 `watch` / `unwatch`。
//!
//! ## 为什么只有这些
//!
//! 纯接口（[`VdfsProvider`] trait 与其域类型）在
//! [`crate::symbio_core::vdfs`]；`vdfs/*` 的**分发**（收集挂载点、
//! 路径解析、树状遍历、事件投递）是 vdfs 插件的职责，见 `plugins/vdfs/host.rs`。
//!
//! 本模块之所以留在 core，是因为**实现方**（如 `plugin_manager` 插件）是插件而非
//! 宿主本身——它需要把宿主句柄从 [`VdfsContext`] 里取出、并把调用宿主服务
//! 得到的 [`PluginError`] 翻回协议错误。让它去依赖 `plugins/vdfs` 会破坏
//! 「插件之间不互相依赖」的分层。
//!
//! [`VdfsProvider`]: crate::symbio_core::vdfs::VdfsProvider

use super::{VdfsChange, VdfsChangeSink, VdfsContext, VdfsError, VdfsResult, VdfsValidationError};
use crate::symbio_core::{PluginError, PluginInvokeRequest};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// [`VdfsError`] → [`PluginError`]
///
/// `Invalid` 的结构化载荷（字段级错误）序列化为 JSON 置于错误文案位，
/// 宿主前端可解析后逐字段高亮；解析失败则按纯文本展示（向前兼容）。
impl From<VdfsError> for PluginError {
    fn from(e: VdfsError) -> Self {
        match e {
            VdfsError::NotFound(m) => PluginError::NotFound(m),
            VdfsError::NotImplemented => PluginError::NotImplemented,
            VdfsError::Invalid(v) => {
                let text = serde_json::to_string(&v).unwrap_or_else(|_| v.message.clone());
                PluginError::ValidationError(text)
            }
            VdfsError::Forbidden(m) => PluginError::Forbidden(m),
            // 宿主错误体系无 Conflict 变体：冲突对使用者同样是「输入不满足前提」，
            // 归入 ValidationError 以便前端在同一处提示
            VdfsError::Conflict(m) => PluginError::ValidationError(format!("冲突：{m}")),
            VdfsError::Internal(m) => PluginError::InternalError(m),
        }
    }
}

/// [`PluginError`] → [`VdfsError`]（provider 调宿主服务后翻译回协议错误）
pub fn from_plugin_error(e: PluginError) -> VdfsError {
    match e {
        PluginError::NotFound(m) => VdfsError::NotFound(m),
        PluginError::NotImplemented => VdfsError::NotImplemented,
        PluginError::ValidationError(m) => match serde_json::from_str::<VdfsValidationError>(&m) {
            Ok(v) => VdfsError::Invalid(v),
            Err(_) => VdfsError::Invalid(VdfsValidationError::new(m)),
        },
        PluginError::Forbidden(m) => VdfsError::Forbidden(m),
        PluginError::InternalError(m) => VdfsError::Internal(m),
        other => VdfsError::Internal(other.to_string()),
    }
}

/// 把 symbio 请求上下文装进不透明 [`VdfsContext`]
pub fn vdfs_context(ctx: &Arc<dyn PluginInvokeRequest>) -> VdfsContext {
    VdfsContext::new(ctx.clone())
}

/// 从 [`VdfsContext`] 取回 symbio 请求上下文
pub fn host_ctx(ctx: &VdfsContext) -> VdfsResult<Arc<dyn PluginInvokeRequest>> {
    ctx.require::<Arc<dyn PluginInvokeRequest>>().cloned()
}

// ==================== 变更订阅表 ====================

/// 被订阅路径 → 引用计数 + 投递器；变更**同步**投递，不经后台任务。
///
/// ## 为什么「一条变更只出总线一次」是必须的
///
/// 广播源是整棵子树共用的（[`notify_change`] 不按路径分流），而投递的终点是**一个
/// 全局广播出口**（`plugins/vdfs/host.rs::event_bus_sink` → `EventBus::try_publish`
/// 推给全部前端连接）。因此两条**重叠**的订阅（会话清单订 `<根>/session`、转写订
/// `<根>/session/<id>/message`）若各投一次，同一条变更就会在总线上出现两次——
/// 前端把它当两条变更各落地一次，流式正文当场叠字。
///
/// 所以本表把投递收敛成**恰好一次**：命中多条相关订阅时只调用**一个**投递器。
/// 至于是哪一个，机制上**无所谓**（本表里的投递器行为相同）；取最长匹配只是让
/// 「同一输入给同一输出」，好让诊断与测试可复现。这与订阅的条数、重叠方式无关。
///
/// ## 为什么是同步投递而不是「广播通道 + 转发任务」
///
/// `tokio::sync::broadcast` 会**静默丢帧**（通道满时慢消费者收到 `Lagged`），
/// 而转写的 `delta` 增量**不可丢**——丢一帧，前端就永久少一段正文。同步投递没有
/// 缓冲，也就不存在丢帧；投递器只做「转成总线事件并 publish」，是非阻塞的，
/// 持锁调用也不会死锁。（背压由下游 `EventBus::try_publish` 负责：通道满时丢帧
/// 但**保留订阅**并补送 resync 指令，见 `symbio_core::event_bus`。）
///
/// ## 「相关」= 同一子树（自身 / 祖先 / 后代）
///
/// 订阅 `<dir>` 的消费者既关心 `<dir>` 子树内的变更（列表项增删），也关心
/// `<dir>` 的祖先（父目录被改名 / 删除时，本视图的整个上下文都没了）。
/// 文件浏览器的 `affects()` 就是这个判定，这里把它下沉到机制层，provider
/// 不必各自实现路径分流。
///
/// ## 为什么是引用计数
///
/// `watch` / `unwatch` 由**视图生命周期**驱动，而两个视图可以订同一个路径
/// （两个窗口 / 两个组件）。计数让「取消」只关掉自己那一层：归零才真正摘掉。
#[derive(Default)]
pub struct VdfsChangeSubscriptions {
    /// 被订阅路径 → （引用计数, 投递器）。投递器以**最新**登记的那条为准。
    /// 包在 `Arc` 里是为了让 `Clone` 得到**同一张表**的句柄（见下方 `impl Clone`）。
    by_path: Arc<Mutex<HashMap<String, (u32, VdfsChangeSink)>>>,
}

/// 变更路径 `changed` 是否落在订阅路径 `subscribed` 的同一子树内
///
/// 空串是 provider 根（`CompositeVdfs::resolve` 对 `<根>/<挂载名>` 给出的 `rel`
/// 就是空串），恒命中。
fn related(subscribed: &str, changed: &str) -> bool {
    subscribed.is_empty()
        || subscribed == changed
        || changed.starts_with(&format!("{subscribed}/"))
        || subscribed.starts_with(&format!("{changed}/"))
}

/// 克隆得到的是**同一张表**（共享 `Arc`）——变更源与它的旁路（如工作目录监听
/// 任务）各自持一份句柄、投递进同一批订阅者，正是引用计数表要支持的用法。
impl Clone for VdfsChangeSubscriptions {
    fn clone(&self) -> Self {
        Self {
            by_path: self.by_path.clone(),
        }
    }
}

impl VdfsChangeSubscriptions {
    /// 登记一条订阅；返回 `true` 表示这是该路径的**首个**订阅者
    /// （调用方可据此做一次性副作用，如接入文件系统监听）。
    pub fn watch(&self, path: &str, sink: VdfsChangeSink) -> bool {
        let mut subs = self.by_path.lock().unwrap();
        match subs.get_mut(path) {
            Some((count, slot)) => {
                *count += 1;
                *slot = sink;
                false
            }
            None => {
                subs.insert(path.to_string(), (1, sink));
                true
            }
        }
    }

    /// 解除一条订阅；返回 `true` 表示该路径**已无**订阅者
    pub fn unwatch(&self, path: &str) -> bool {
        let mut subs = self.by_path.lock().unwrap();
        match subs.get_mut(path) {
            None => false,
            Some((count, _)) => {
                *count = count.saturating_sub(1);
                if *count > 0 {
                    false
                } else {
                    subs.remove(path);
                    true
                }
            }
        }
    }

    /// 投递一条变更：**恰好一次**。
    ///
    /// 命中多条相关订阅时只调用一个投递器——**不是**「挑最具体的那个消费者」，
    /// 而是「这条变更只能出总线一次」：本表里的投递器都是同一个广播出口，
    /// 多调一次就是同一条变更在总线上出现两次（前端会各落地一次，流式正文叠字）。
    ///
    /// 取最长匹配只为**确定性**（同一输入永远给同一输出，诊断与测试可复现），
    /// 不代表路径更具体的那条订阅「更该收到」。无订阅者时直接返回。
    pub fn notify(&self, change: &VdfsChange) {
        let subs = self.by_path.lock().unwrap();
        if subs.is_empty() {
            return;
        }
        // 同一路径的引用计数共用一个投递器，故命中一条即是「恰好一次」
        let hit = subs
            .iter()
            .filter(|(path, _)| related(path, &change.path))
            .max_by_key(|(path, _)| path.len());
        if let Some((_, (_, sink))) = hit {
            let sink = sink.clone();
            // 投递在提前 return 前克隆好，锁在此处已释放（sink 不回调本表，
            // 但保持「锁内只挑订阅者」的纪律，未来加逻辑也不会引入死锁）
            drop(subs);
            sink(change.clone());
        }
    }

    /// 当前订阅者总数（含同一路径的重复计数）
    pub fn subscriber_count(&self) -> u32 {
        self.by_path
            .lock()
            .unwrap()
            .values()
            .map(|(count, _)| *count)
            .sum()
    }

    /// 是否还有任何订阅者（provider 可据此决定是否停掉昂贵的上游监听）
    pub fn has_subscribers(&self) -> bool {
        !self.by_path.lock().unwrap().is_empty()
    }

    /// 当前被订阅路径（诊断与测试用）
    pub fn paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_path.lock().unwrap().keys().cloned().collect();
        v.sort();
        v
    }
}

/// 自管变更源的 provider（session / agent 目录）直接持有一份表实例，
/// 不必经全局注册表；集中式存储的 provider 只有访问层句柄、拿不到 provider
/// 实例，故按 `kind` 共享一张全局表。两种用法共用同一套投递纪律。
///
/// 按**类型**（`kind`）而非 provider 实例持有——同一类型的 provider 可能被多次
/// 构造（每次 `traverse` 一份），共享同一张表才能让订阅与投递天然配对。
fn hub_of(kind: &str) -> Arc<VdfsChangeSubscriptions> {
    static HUBS: OnceLock<Mutex<HashMap<String, Arc<VdfsChangeSubscriptions>>>> = OnceLock::new();
    let hubs = HUBS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut m = hubs.lock().unwrap();
    if let Some(h) = m.get(kind) {
        return h.clone();
    }
    let hub = Arc::new(VdfsChangeSubscriptions::default());
    m.insert(kind.to_string(), hub.clone());
    hub
}

/// 广播一次变更（订阅方据此刷新，**非轮询**）。无订阅者时直接返回。
///
/// 由集中式存储实现（`crate::providers::vdfs_service` 的三种拓扑）与目录自管型
/// provider（agent 目录）在写 / 删成功后调用。
///
/// **它只发无载荷变更**（`VdfsChange::bare`）——绝大多数资源信号长这样。带业务
/// 载荷的变更（消息帧 / 节点视图）由**生产者直接经它已持有的订阅表**投递：
/// `VdfsChangeSubscriptions::notify(&VdfsChange::with_data(path, data))`，见
/// `session::transcript::Transcript::publish`。这里**刻意不提供**对称的
/// `notify_change_with_data` 门面：它没有生产者（带载荷的只有会话域，而会话域
/// 拿的是订阅表本身），而留一个没人调用的「能力」比没有更糟——文档会照着它写，
/// 读者会以为存在第二条投递路径。
pub fn notify_change(kind: &str, path: &str) {
    hub_of(kind).notify(&VdfsChange::bare(path));
}

/// 订阅某挂载点的变更（`VdfsProvider::watch` 的实现体）；变化发生时调用 `sink`。
pub async fn watch_changes(kind: &str, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
    hub_of(kind).watch(path, sink);
    Ok(())
}

/// 取消订阅（`VdfsProvider::unwatch` 的实现体；与 [`watch_changes`] 严格配对）
pub async fn unwatch_changes(kind: &str, path: &str) -> VdfsResult<()> {
    hub_of(kind).unwatch(path);
    Ok(())
}

// ==================== 变更订阅表 ====================

#[cfg(test)]
#[path = "host.test.rs"]
mod tests;
