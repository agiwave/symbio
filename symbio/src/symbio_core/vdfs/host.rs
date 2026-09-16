//! VDFS 宿主桥（symbio 侧）—— 把纯接口接到 symbio 类型上
//!
//! ⚠️ 这是 core 中**唯一**依赖宿主的部分。它只做三件事：
//!
//! 1. **上下文注入**：`Arc<dyn InvokeRequest>` ↔ [`VdfsContext`]
//!    （provider 经 `ctx.require::<Arc<dyn InvokeRequest>>()` 取回宿主句柄）；
//! 2. **错误翻译**：[`VdfsError`] ↔ [`PluginError`] 双向映射；
//! 3. **变更广播**：挂载点写 / 删后 [`notify_change`]，`watch` 经
//!    [`watch_changes`] 订阅后转发——前端因此无需轮询（**非**轮询实现）。
//!    转发由 [`ChangeSubscriptions`] 统一收敛：**重叠订阅不会重复投递**
//!    （每条变更只投给最具体的那条相关订阅），同一路径的多位订阅者
//!    按引用计数配对 `watch` / `unwatch`。
//!
//! ## 为什么只有这些
//!
//! 纯接口（[`VdfsProvider`] trait 与其域类型）在
//! [`crate::symbio_core::vdfs_provider`]；`vdfs/*` 的**分发**（收集挂载点、
//! 路径解析、树状遍历、事件投递）是 vdfs 插件的职责，见 `plugins/vdfs/host.rs`。
//!
//! 本模块之所以留在 core，是因为**实现方**（如 `setting` 插件）是插件而非
//! 宿主本身——它需要把宿主句柄从 [`VdfsContext`] 里取出、并把调用宿主服务
//! 得到的 [`PluginError`] 翻回协议错误。让它去依赖 `plugins/vdfs` 会破坏
//! 「插件之间不互相依赖」的分层。
//!
//! [`VdfsProvider`]: crate::symbio_core::vdfs_provider::VdfsProvider

use crate::symbio_core::vdfs_provider::{
    VdfsChange, VdfsChangeSink, VdfsContext, VdfsError, VdfsResult, VdfsValidationError,
};
use crate::symbio_core::{InvokeRequest, PluginError};
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
pub fn vdfs_context(ctx: &Arc<dyn InvokeRequest>) -> VdfsContext {
    VdfsContext::new(ctx.clone())
}

/// 从 [`VdfsContext`] 取回 symbio 请求上下文
pub fn host_ctx(ctx: &VdfsContext) -> VdfsResult<Arc<dyn InvokeRequest>> {
    ctx.require::<Arc<dyn InvokeRequest>>().cloned()
}

// ==================== 变更订阅表 ====================

/// 被订阅路径 → 引用计数 + 投递器；变更**同步**投递，不经后台任务。
///
/// ## 为什么不是「每个被订阅路径一个转发任务 + 一条广播」
///
/// 广播源是整棵子树共用的（[`notify_change`] 不按路径分流）。若每个路径各起
/// 一个任务，两条**重叠**的订阅（会话清单订 `.vdfs/session`、转写订
/// `.vdfs/session/<id>/消息`）就会把同一条变更投到总线上两次——前端收到重复帧，
/// 流式文本叠字。本表把投递收敛成**恰好一次**：每条变更只投给与之相关的最具体
/// 的那条订阅，与订阅的条数、重叠方式都无关。
///
/// ## 为什么是同步投递而不是「广播通道 + 转发任务」
///
/// `tokio::sync::broadcast` 会**静默丢帧**（通道满时慢消费者收到 `Lagged`），
/// 而转写的 `appended` 增量**不可丢**——丢一帧，前端就永久少一段正文，且没有
/// 任何机制会纠正（两条链路互不校验）。同步投递没有缓冲，也就不存在丢帧；
/// 投递器只做「转成总线事件并 publish」，是非阻塞的，持锁调用也不会死锁。
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
pub struct ChangeSubscriptions {
    /// 被订阅路径 → （引用计数, 投递器）。投递器以**最新**登记的那条为准。
    /// 包在 `Arc` 里是为了让 `Clone` 得到**同一张表**的句柄（见下方 `impl Clone`）。
    by_path: Arc<Mutex<HashMap<String, (u32, VdfsChangeSink)>>>,
}

/// 变更路径 `changed` 是否落在订阅路径 `subscribed` 的同一子树内
///
/// 空串是 provider 根（`CompositeVdfs::resolve` 对 `.vdfs/<挂载名>` 给出的 `rel`
/// 就是空串），恒命中。
fn related(subscribed: &str, changed: &str) -> bool {
    subscribed.is_empty()
        || subscribed == changed
        || changed.starts_with(&format!("{subscribed}/"))
        || subscribed.starts_with(&format!("{changed}/"))
}

/// 克隆得到的是**同一张表**（共享 `Arc`）——变更源与它的旁路（如工作目录监听
/// 任务）各自持一份句柄、投递进同一批订阅者，正是引用计数表要支持的用法。
impl Clone for ChangeSubscriptions {
    fn clone(&self) -> Self {
        Self {
            by_path: self.by_path.clone(),
        }
    }
}

impl ChangeSubscriptions {
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

    /// 投递一条变更：只给**最具体**的那条相关订阅（最长匹配），因此重叠订阅
    /// 不会收到重复帧。无订阅者时直接返回。
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

/// 自管变更源的 provider（session / agent bundle）直接持有一份表实例，
/// 不必经全局注册表；集中式存储的 provider 只有访问层句柄、拿不到 provider
/// 实例，故按 `kind` 共享一张全局表。两种用法共用同一套投递纪律。
///
/// 按**类型**（`kind`）而非 provider 实例持有——同一类型的 provider 可能被多次
/// 构造（每次 `traverse` 一份），共享同一张表才能让订阅与投递天然配对。
fn hub_of(kind: &str) -> Arc<ChangeSubscriptions> {
    static HUBS: OnceLock<Mutex<HashMap<String, Arc<ChangeSubscriptions>>>> = OnceLock::new();
    let hubs = HUBS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut m = hubs.lock().unwrap();
    if let Some(h) = m.get(kind) {
        return h.clone();
    }
    let hub = Arc::new(ChangeSubscriptions::default());
    m.insert(kind.to_string(), hub.clone());
    hub
}

/// 广播一次变更（订阅方据此刷新，**非轮询**）。无订阅者时直接返回。
///
/// 由集中式存储实现（`crate::providers::vdfs_service` 的三种拓扑）与目录自管型
/// provider（agent bundle）在写 / 删成功后调用。
pub fn notify_change(kind: &str, path: &str, change: &str) {
    hub_of(kind).notify(&VdfsChange::new(path, change));
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
mod tests {
    use super::*;

    #[test]
    fn error_translation_roundtrip() {
        let e = VdfsError::NotFound("x".into());
        assert!(matches!(PluginError::from(e), PluginError::NotFound(_)));
        assert!(matches!(
            PluginError::from(VdfsError::NotImplemented),
            PluginError::NotImplemented
        ));
        assert!(matches!(
            from_plugin_error(PluginError::Forbidden("f".into())),
            VdfsError::Forbidden(_)
        ));

        // Conflict 归入 ValidationError（宿主无 Conflict 变体）
        assert!(matches!(
            PluginError::from(VdfsError::Conflict("dup".into())),
            PluginError::ValidationError(_)
        ));

        // 字段级校验载荷 → 错误文案位 JSON → 可解析还原
        let payload = VdfsValidationError::new("坏").with_field("port", "越界");
        let text = match PluginError::from(VdfsError::Invalid(payload.clone())) {
            PluginError::ValidationError(t) => t,
            other => panic!("应为校验错误，实为 {other:?}"),
        };
        assert_eq!(
            from_plugin_error(PluginError::ValidationError(text)),
            VdfsError::Invalid(payload)
        );
    }

    /// provider 侧取回宿主句柄：类型匹配给 `Some`，不匹配报 InternalError
    #[test]
    fn host_ctx_downcasts_invoke_request() {
        use crate::symbio_core::SimpleRequest;
        let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let vctx = vdfs_context(&host);
        let back = host_ctx(&vctx).expect("应取回宿主句柄");
        assert!(Arc::ptr_eq(&host, &back));

        // 宿主句柄不可 `Debug`，故不能用 `unwrap_err`（它要求 Ok 侧可 Debug）
        let err = match host_ctx(&VdfsContext::new(1u8)) {
            Ok(_) => panic!("宿主类型不匹配时应报错"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }

    /// 同一路径重复订阅：计数累加，取消一次仍在（其余订阅者不受影响）
    #[tokio::test]
    async fn repeated_watch_on_same_path_is_counted() {
        let subs = ChangeSubscriptions::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..2 {
            let s = seen.clone();
            subs.watch(
                "a",
                Arc::new(move |c: VdfsChange| s.lock().unwrap().push(c.path)),
            );
        }
        subs.notify(&VdfsChange::new("a", "updated"));
        assert_eq!(subs.subscriber_count(), 2);

        subs.unwatch("a");
        subs.notify(&VdfsChange::new("a", "updated"));
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &["a".to_string(), "a".to_string()],
            "两位订阅者各收一次；取消一位后不再投递"
        );

        subs.unwatch("a");
        assert_eq!(subs.subscriber_count(), 0);
        assert!(!subs.has_subscribers());
    }

    /// **重叠订阅不重复投递**：会话清单订根、转写订其子树，
    /// 一条消息变更只到达最具体的那条订阅 ⇒ 前端不会叠字。
    #[tokio::test]
    async fn overlapping_subscriptions_deliver_once() {
        let subs = ChangeSubscriptions::default();
        let broad = Arc::new(Mutex::new(Vec::new()));
        let narrow = Arc::new(Mutex::new(Vec::new()));
        {
            let b = broad.clone();
            subs.watch(
                "",
                Arc::new(move |c: VdfsChange| b.lock().unwrap().push(c.path)),
            );
            let n = narrow.clone();
            subs.watch(
                "abc/消息",
                Arc::new(move |c: VdfsChange| n.lock().unwrap().push(c.path)),
            );
        }
        subs.notify(&VdfsChange::new("abc/消息/m1", "appended"));
        subs.notify(&VdfsChange::new("xyz", "updated"));

        assert_eq!(narrow.lock().unwrap().as_slice(), ["abc/消息/m1"]);
        assert_eq!(broad.lock().unwrap().as_slice(), ["xyz"]);
    }

    /// 无订阅者时 `notify` 直接返回（不遍历、不投递）
    #[tokio::test]
    async fn notify_without_subscribers_is_noop() {
        let subs = ChangeSubscriptions::default();
        subs.notify(&VdfsChange::new("a", "created"));
        assert_eq!(subs.subscriber_count(), 0);
    }
}
