//! 会话**收件箱**：用户消息入队 + 进程内消费。
//!
//! ## 它解决什么
//!
//! `chat/send` 把「接收消息」与「跑一轮」绑在一起，并且要有一个**调用方连接**在
//! 那儿等结果。子智能体空间没有这样的调用方——它的会话照样要能完整地跑。于是
//! 用户消息改以**对空间的写入**表达：往 `<sid>/inbox` 写一条即入队，由空间自己
//! 消费（D-026）。顶层会话走同一条路，只是恰好也有人替它
//! 调 `chat/send`——`chat/send` 因此退化成「入队」。
//!
//! ## 一条消息的两个落点
//!
//! | 阶段 | 住在 | 谁写 |
//! |---|---|---|
//! | 已入队、未消费 | [`InboxItem`]（本模块的队列）→ VDFS `<sid>/inbox/<iid>` | [`SessionPlugin::enqueue_inbox`] |
//! | 已消费 | 转写（存储 + `Transcript`） | 统一发送链路（`orchestrator/entry.rs::start_turn`） |
//!
//! 条目**出队即消失**：它不是"被处理过的队列项"，而是"还没成为消息的消息"。
//! 「正在处理的那条」在转写里，不在队列里——所以取消它走中止
//! （`action(<会话地址>, "abort")`），而不是删队列项。
//!
//! ## 消费规则（与 D-026 的三条决定一一对应）
//!
//! 1. **FIFO**：同一个会话严格按入队顺序，先到先跑；
//! 2. **忙则排队，不抢占**：会话正在跑（`is_working`）时只把条目留在队里，等这一轮
//!    收尾再取下一条——不合并、不中断、不并发。抢占（挂起在跑任务）今天**没有实现**：
//!    判定面已删（`D-388`），插话一律排队到轮末；
//! 3. **机制统一**：顶层会话与子智能体空间共用本模块，没有"子智能体专用分支"。
//!
//! ## 为什么是**每插件实例一个**消费者
//!
//! 消费者在 [`SessionPlugin::build`] 里起一次，扫过本实例名下所有会话的队列。
//! 不按会话起任务，是因为"按会话起"必须由写入方触发，而写入方（provider 的
//! `write`）手上只有 `&self`——拿不到 `Arc<SessionPlugin>` 就 spawn 不出消费者，
//! 只能靠"等某个恰好持有 Arc 的时机"来补，于是"第一次写入能不能被消费"取决于
//! 装配顺序（脆弱且不可测）。实例级消费者在构造点起，**早于任何写入**，与谁写、
//! 什么时候写无关。

use super::super::active::{ActiveSessionState, InboxItem};
use super::super::plugin::{inbox_item_node, inbox_item_path, SessionConfig, SessionPlugin};
use std::sync::Arc;
use symbio_kernel::chat::chat_node as cm;
use symbio_kernel::vdfs::VdfsChange;
use symbio_sdk::chat::session_chat;
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;

/// 抽干上界：一次取几条。
///
/// `supplements_enabled = false` 时退化为「取一条」——与下游的合并叠加后恰好等于
/// 「一条消息 = 一轮」的今天行为。
///
/// 两个抽干点（收件箱消费者的异步取批、轮边界的同步取批）共用这一个判定：
/// 各写一遍必然在「上界取谁」上漂移。
fn drain_limit(cfg: &SessionConfig) -> usize {
    if cfg.supplements_enabled {
        cfg.supplements_max_per_drain.max(1)
    } else {
        1
    }
}

/// 抽干之后逐条发删除变更——**抽干与通知成对**：分开写就会出现「抽了没通知」
/// （前端以为条目还在）或「通知了没抽」（条目凭空消失）两种静默不一致。
fn announce_drained(change_subs: &VdfsChangeSubscriptions, session_id: &str, batch: &[InboxItem]) {
    for item in batch {
        change_subs.notify(&VdfsChange::bare(inbox_item_path(session_id, &item.id)));
    }
}

/// **忙窗批次边界**抽干（同步）：只在该会话**正在跑**时取走队首一批。
///
/// 忙门方向是它与轮收尾抽干口（[`SessionPlugin::take_inbox_batch_when_idle`]）的
/// 唯一区别：批次边界发生在**本轮之内**（工具结果落格之后、下一次请求装配之前），
/// 因此先确认本会话确实在跑；空闲时取走的那批属于**下一轮**（轮收尾的 A 线），
/// 两条抽干点不能互相顶替。
///
/// `supplements_enabled = false` ⇒ 空批：关闭补充整合等于「一条消息 = 一轮」，
/// 忙窗写入留队，由轮收尾各自开轮（与改造前逐字一致）。
///
/// ⚠️ 调用方负责 `block_in_place`：单线程 runtime 的 async 上下文里
/// `blocking_read` 会 panic。
pub(crate) fn take_busy_supplement_batch_sync(
    config: &tokio::sync::RwLock<SessionConfig>,
    change_subs: &VdfsChangeSubscriptions,
    state: &Arc<ActiveSessionState>,
) -> Vec<InboxItem> {
    let limit = {
        let cfg = config.blocking_read();
        if !cfg.supplements_enabled {
            return Vec::new();
        }
        drain_limit(&cfg)
    };

    let batch: Vec<InboxItem> = {
        let mut inner = state.inner.blocking_write();
        // 空闲留给轮收尾：它按「下一轮」的语义抽干，不在这里折进本轮。
        if !inner.is_working() {
            return Vec::new();
        }
        let n = inner.inbox.len().min(limit);
        inner.inbox.drain(..n).collect()
    };

    announce_drained(change_subs, &state.session_id, &batch);
    batch
}

// ── 收件箱写口 / 抽干口 ───────────────────────────────────

impl SessionPlugin {
    /// **入队**一条用户消息（收件箱集合的唯一写入口）。
    ///
    /// 同时做两件事，顺序固定：入队 → 发变更 → 唤醒消费者。变更落在**条目自身**的
    /// 地址上（`<sid>/inbox/<iid>`）并带条目节点视图，因此订阅方零回读即可显示
    /// "有一条待发的消息"（与 D-025 的落点约定一致）。
    ///
    /// ## 条目 id 的三个来源（优先级从高到低）
    ///
    /// 1. **地址末段**（`write(<sid>/inbox/<iid>)` 的 `iid`）——地址即身份；
    /// 2. **消息自带 id**（`chat/send` 带的是客户端生成的 id）。保留它是**必须的**：
    ///    前端先按自己的 id 放了乐观副本，落库回包若换了 id，那条副本就永远等不到
    ///    替身，界面上会变成两条；
    /// 3. `turn::llm_short_id()`（都没有时生成）——与消息 id 同一套格式，地址末段、
    ///    日志、前端 key 都读它。
    ///
    /// 三条都收在这一个地方：分散到调用点就会出现"同一个地址取到两个 id"。
    pub(crate) async fn enqueue_inbox(
        &self,
        session_id: &str,
        id: Option<String>,
        message: cm::ChatNode,
        params: session_chat::Request,
        workdir: Option<String>,
    ) -> InboxItem {
        let mut message = message;
        let id = id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| (!message.id.trim().is_empty()).then(|| message.id.clone()))
            .unwrap_or_else(symbio_sdk::llm::turn::llm_short_id);
        // 消息 id 与条目 id **是同一个值**：地址末段即身份，两处各生成一个会让
        // 「按地址取消息」在两套 id 之间对不上（见 `message_path` 的同款约定）。
        message.id = id.clone();
        let item = InboxItem {
            id: id.clone(),
            message,
            params,
            workdir,
        };

        let state = self.active_mgr.get_or_create(session_id).await;
        state.inner.write().await.inbox.push_back(item.clone());
        self.change_subs.notify(&VdfsChange::with_data(
            inbox_item_path(session_id, &id),
            inbox_item_node(&item),
        ));
        item
    }

    /// 取消一条**仍在排队**的条目（`delete(<sid>/inbox/<iid>)` 的实现）。
    ///
    /// 已出队（正在处理）的条目**找不到**：它已经不是队列项了。取消正在跑的那条
    /// 是**中止**（`chat/abort`）——两种取消作用在不同对象上，因此是两个动作，
    /// 不硬塞进一个动词。返回 `false` 表示"队里没有这一条"，由调用方翻成
    /// `NotFound`（不静默成功）。
    pub(crate) async fn cancel_inbox_item(&self, session_id: &str, iid: &str) -> bool {
        let state = self.active_mgr.get_or_create(session_id).await;
        let removed = {
            let mut inner = state.inner.write().await;
            match inner.inbox.iter().position(|i| i.id == iid) {
                Some(pos) => inner.inbox.remove(pos).is_some(),
                None => false,
            }
        };
        if removed {
            self.change_subs
                .notify(&VdfsChange::bare(inbox_item_path(session_id, iid)));
        }
        removed
    }

    /// 清空队列（`action(<sid>/inbox, "clear")` 的实现）——返回被取消的条目 id。
    ///
    /// 与会话转写的 `clear` 同款语义：容器（会话）保留，集合清空。两者都被表达成
    /// **逐条移除**（每条落在自己的地址上），因此消费端不必认识"集合级删除"。
    pub(crate) async fn clear_inbox(&self, session_id: &str) -> Vec<String> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let ids: Vec<String> = {
            let mut inner = state.inner.write().await;
            inner.inbox.drain(..).map(|i| i.id).collect()
        };
        for id in &ids {
            self.change_subs
                .notify(&VdfsChange::bare(inbox_item_path(session_id, id)));
        }
        ids
    }

    /// 该会话当前**待消费**的条目（VDFS `<sid>/inbox` 的数据源）
    pub(crate) async fn inbox_items(&self, session_id: &str) -> Vec<InboxItem> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let inner = state.inner.read().await;
        inner.inbox.iter().cloned().collect()
    }

    /// 只在该会话**空闲**时取走队首一批（至多 `supplements_max_per_drain` 条），
    /// 并逐条发删除变更——轮收尾后的抽干口。
    ///
    /// 那道忙门是必须的：抽干与开下一轮是同一件事的两半，若抽干时本轮还在跑，
    /// 抽出来的那一批会在另一条轮上再开一次——同一份存储出现两个写方。
    /// 忙则返回空批（条目留在队里）。
    ///
    /// 判空、取批与变更通知在**同一处**完成（[`announce_drained`]）：调用方拿到的批
    /// 一定已从队列里消失，也一定已经投递过删除变更。分开写就会出现"抽了没通知"
    /// （前端以为条目还在）或"通知了没抽"（条目凭空消失）两种静默不一致。
    pub(crate) async fn take_inbox_batch_when_idle(
        &self,
        state: &Arc<ActiveSessionState>,
    ) -> Vec<InboxItem> {
        let limit = {
            let cfg = self.config.read().await;
            drain_limit(&cfg)
        };

        let batch: Vec<InboxItem> = {
            let mut inner = state.inner.write().await;
            if inner.is_working() {
                return Vec::new();
            }
            let n = inner.inbox.len().min(limit);
            inner.inbox.drain(..n).collect()
        };

        announce_drained(&self.change_subs, &state.session_id, &batch);
        batch
    }

    /// 取队首**一条**（一轮的最小单位），并通知这一条已出队。
    ///
    /// 与 [`Self::take_inbox_batch_when_idle`] 的区别只在**数量**：开轮是「一条消息 = 一轮」，
    /// 补充合并是轮末的可选加工。按批取会把合并上界之外的消息一起抽走，
    /// 一条只够开一轮——多抽的那些就再没有开轮的机会了。
    ///
    /// **忙则留队**（D-026 #4）：本轮还在跑时不再开第二轮——那是把「补充」当成
    /// 并发会话，同一份存储会出现两个写方。忙窗写入只入队，出队由轮收尾后的
    /// 抽干接手（[`Self::take_inbox_batch_when_idle`]）。
    pub(crate) async fn take_inbox_front(
        &self,
        state: &Arc<ActiveSessionState>,
    ) -> Option<InboxItem> {
        let item = {
            let mut inner = state.inner.write().await;
            if inner.is_working() {
                return None;
            }
            inner.inbox.pop_front()
        };
        if let Some(item) = &item {
            announce_drained(
                &self.change_subs,
                &state.session_id,
                std::slice::from_ref(item),
            );
        }
        item
    }
}

#[cfg(test)]
#[path = "inbox.test.rs"]
mod tests;
