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
//!    收尾再取下一条——不合并、不中断、不并发；**排队的是插话，不是在跑的任务**：
//!    忙窗里本模块的抢占判定者（[`PreemptionDecider`]）会评估当前任务要不要挂起
//!    ，结论经 [`super::super::active::PreemptPending`] 跨过忙窗、
//!    在空闲分支落成事实。挂起 / 恢复**都不开新轮、都不动队列**，所以与「不抢占」
//!    不冲突（判定顺序见 [DE-016 §9.1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）；
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
use symbio_kernel::actors::{ActorSpec, Scope};
use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::vdfs::VdfsChange;
use symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;
use symbio_sdk::authz::PRINCIPAL_REFLEX;
use symbio_sdk::chat::{chat_node as cm, session_chat};
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;
// 任务族的名字由 owner 插件（`loop`）的清单登记、由生成物投影到这里（DE-024 D-204 / D-206）：
// 本插件是这一族的**第二个消费方**——它写挂起与恢复两格，也按名字读就绪。
use symbio_plugin_api::events::{
    EVENT_TASK_ASSERTED, EVENT_TASK_HELD, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
};

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

/// [`SessionPlugin::take_inbox_batch`] 的**同步**孪生（v2 工具循环的轮边界注入挂点用它）。
///
/// ## 为什么是自由函数、为什么收这三样
///
/// 取一个会话的队首只需要：**上界从哪来**（`config`）、**队列在哪**（`state.inner.inbox`）、
/// **通知投给谁**（`change_subs`）——插件本身在这一步没有贡献。于是它不住在
/// `impl SessionPlugin` 上，需要它的一方（`chat_loop::SupplementDrain`）拿的就是这三样，
/// 而不是整个插件（[D-075](../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P2）。
///
/// ## 上界与通知都不另写一份
///
/// 走的是 [`drain_limit`] 与 [`announce_drained`]——与异步孪生同一条判定，两条路不可能漂。
///
/// ⚠️ 调用方负责 `block_in_place`（见 `SupplementDrain::take_inbox_batch_sync` 的说明）：
/// 单线程 runtime 的 async 上下文里 `blocking_read` 会 panic。
pub(crate) fn take_inbox_batch_sync(
    config: &tokio::sync::RwLock<SessionConfig>,
    change_subs: &VdfsChangeSubscriptions,
    state: &Arc<ActiveSessionState>,
) -> Vec<InboxItem> {
    let limit = {
        let cfg = config.blocking_read();
        drain_limit(&cfg)
    };

    let batch: Vec<InboxItem> = {
        let mut inner = state.inner.blocking_write();
        let n = inner.inbox.len().min(limit);
        inner.inbox.drain(..n).collect()
    };

    announce_drained(change_subs, &state.session_id, &batch);
    batch
}

// ── S07 抢占判定────────────────────────────────────────────
//
// 唯一生产消费方就是本文件的忙窗判定与空闲落格链（D-190）；判定是纯函数，
// 落格是事件，两者各住一边。

/// 抢占判定结论（反射档三选一 + 超时默认，[DE-016 §9.1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preemption {
    /// 当前无在跑任务——插话放行，直接开始新 turn。
    Proceed,
    /// 挂起当前任务：`as_of_seq` 是挂起锚 T（恢复 = 以 T 重放投影，零状态迁移）。
    Suspend { task_id: String, as_of_seq: u64 },
    /// 排队：任务不可中断 / final 已发出（已发出的发言不可撤回）。
    Queue,
    /// 判定超时：**默认「继续」不挂起**——挂起会留下需要恢复的状态，继续不会；
    /// 两个选择都可能错，错的代价不对称，默认选代价小的（DE-016 §9.1）。
    TimeoutDefaultContinue,
}

/// 抢占判定者（[DE-037 §3](../../../../docs/003-design/037-插话与实时打断.md "DE-037-插话与实时打断")）。
///
/// `pattern = decider`、`capabilities = [JudgeIntent]`、`budget_ms = 80`——
/// 系统第一次需要在几十毫秒内对外部信号做决策。**判定者只产控制事件**
/// （`task.controlled`），无 `reply.*` 写权——它不得直接发言（DE-037 §5，由
/// grants 表保证：`authz::ROWS` 里 [`PRINCIPAL_REFLEX`] 那一行，判据见
/// `authz.test.rs::the_reflex_row_is_narrower_than_main`）。
///
/// 能力那一半**不在这里硬写**：`capabilities = [judge.intent]` 的权威是授权表，
/// [`Self::spec`] 从表行取（`production_matrix` ⇒ `grant_names_of`）。两处各写一份能力清单
/// 必然漂移成「表授予 A、闸门判 B」——两边都是合法能力名。
pub struct PreemptionDecider;

impl PreemptionDecider {
    /// S07 抢占判定者**行**（`ActorSpec` 生产构造点）。
    ///
    /// `{ principal: session/reflex, pattern: decider, capabilities: [judge.intent],
    /// budget_ms: 80, scope: root }`——五个字段全部**取实**，与 DE-037 §3 表逐字相符：
    /// principal/pattern/capabilities 同源于授权表那一行（`PRINCIPAL_REFLEX`
    /// 窄行），budget = 反射档，scope = root。
    ///
    /// 判定者必须有**行**：若 `decide` 被直驱、预算由调用方现算、规格从不构造，就是
    /// 「判定真、行假」。本构造点让预算与能力有了随主体走的载体，读方见
    /// [`Self::decide`]（预算）与本文件的写侧闸（能力，`actor_can_judge_intent`）。
    pub fn spec(&self) -> ActorSpec {
        let principal = PRINCIPAL_REFLEX.to_string();
        ActorSpec {
            pattern: symbio_sdk::actors::pattern_for_principal(&principal),
            capabilities: symbio_sdk::authz::production_matrix().grant_names_of(&principal),
            budget_ms: LatencyTier::Reflex.budget_ms(),
            scope: Scope::Root,
            principal,
        }
    }

    /// 对当前在跑任务的处置判定。
    ///
    /// `spec`：判定者**自己的行**（[`Self::spec`]）。预算从它身上读
    /// （`spec.budget_ms`）——不再是调用方现算一个裸 `u64` 塞进来，
    /// 「判定者行声明了多少预算」因此是一等数据、有真读方。
    ///
    /// `elapsed_ms`：判定者自身耗时（调用方 `Instant` 计时后传入——判定是纯函数，
    /// 计时留在边界上）。预算内（≤ `budget_ms`）才做实质判定，超时走默认分支。
    /// 判定顺序（DE-016 §9.1）：无在跑任务 → 放行；final 已发出 → 排队；否则 → 挂起。
    ///
    /// 能力（`judge.intent`）**不在本函数判**：它是**写权**，拦在落格那一侧
    /// （[`SessionPlugin::commit_preemption`] 的写侧闸），与 `decide` 是纯函数判定分开——
    /// 判定可以算，写控制事实才需要资格。
    pub fn decide(&self, spec: &ActorSpec, events: &[Event], elapsed_ms: u64) -> Preemption {
        if elapsed_ms > spec.budget_ms {
            return Preemption::TimeoutDefaultContinue;
        }
        // 在跑任务 = 最新一个 opened 且未终态、未挂起的任务。
        let mut suspended: Vec<&str> = Vec::new();
        let mut done: Vec<&str> = Vec::new();
        let mut running: Option<(&str, u64)> = None;
        for e in events {
            let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
                continue;
            };
            if e.entity == Entity::Task {
                match e.kind.as_str() {
                    EVENT_TASK_OPENED => {
                        running = Some((id, e.seq.map(|s| s.value()).unwrap_or(u64::MAX)));
                    }
                    EVENT_TASK_ASSERTED => {
                        done.push(id);
                        suspended.retain(|x| *x != id);
                    }
                    EVENT_TASK_HELD => suspended.push(id),
                    EVENT_TASK_PROGRESS => {
                        suspended.retain(|x| *x != id);
                    }
                    _ => {}
                }
            }
        }
        let Some((task_id, opened_seq)) =
            running.take_if(|(id, _)| !done.contains(id) && !suspended.contains(id))
        else {
            return Preemption::Proceed;
        };
        // final 已发出（任务开启后有过收束发言）→ 排队：已发出的发言不可撤回。
        let final_after_task = events.iter().any(|e| {
            e.kind == EVENT_CHAT_ASSISTANT_FINAL
                && e.seq.map(|s| s.value()).unwrap_or(u64::MAX) > opened_seq
        });
        if final_after_task {
            return Preemption::Queue;
        }
        Preemption::Suspend {
            task_id: task_id.to_string(),
            as_of_seq: opened_seq,
        }
    }

    /// 挂起事件（`task × held`）——挂起就是一条事件，不需要新状态机。
    ///
    /// 两个锚各司其职，别混：
    /// - `source_seq` = **挂起前最后一条已落事实的 seq**（DE-016 §9.2「记录当前
    ///   seq 为 T」）——它是幂等键的后半段，因此必须**每次挂起都不同**：同一个任务
    ///   可以被挂起多次（挂起 → 恢复 → 再挂起），拿任务开格 seq 当锚会让第二次撞
    ///   `Duplicate` 而**静默丢掉**那次挂起（J3）。
    ///
    ///   **不能取 `head`**：`head` 是"下一条的号"，它还不是事实。拿它当
    ///   `produced_by` 会让落格后那条边指向**自己**（`produced_by == seq`）。
    ///   I2 只查"有没有边"时这是绿的，`provenance_dangling` 上线后它必须红——
    ///   调用方因此传 `head - 1`（见 [`SessionPlugin::append_preemption`]）。
    /// - `as_of_seq` = 任务开格 seq（[`Self::decide`] 给出的 as-of 重放锚，DE-016 §9.2）——它是判据的一部分，随事实一起落盘，不留在内存里。
    pub fn held_event(&self, task_id: &str, source_seq: u64, as_of_seq: u64) -> Event {
        Event::pending(
            format!("held-{task_id}-{source_seq}"),
            EVENT_TASK_HELD,
            Entity::Task,
            Verb::Held,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id, "as_of": as_of_seq }))
    }

    /// 恢复事件（`task × progressed`）——挂起的逆操作，同样是一条事件。
    ///
    /// 与 [`Self::held_event`] 成对：**只挂不收就是永久挂起**（挂起期间任务不进
    /// `readyset`，而 `readyset` 又是模型唯一的调度候选来源 ⇒ 任务从此再也不被
    /// 调度，且没有任何一条消息说得出为什么）。故插话轮一结束就由调用方写本条。
    /// 锚同样取写入时刻的 head：同一任务的第二次挂起要有第二次恢复。
    pub fn resume_event(&self, task_id: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("res-{task_id}-{source_seq}"),
            EVENT_TASK_PROGRESS,
            Entity::Task,
            Verb::Progressed,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "task_id": task_id }))
    }

    /// 控制事件（`control × opened`）——打断处置的产出事实。
    pub fn control_event(&self, reason: &str, source_seq: u64) -> Event {
        Event::pending(
            format!("ctrl-{source_seq}"),
            symbio_plugin_api::events::EVENT_TASK_CONTROLLED,
            Entity::Control,
            Verb::Opened,
            0,
            PRINCIPAL_REFLEX,
        )
        .with_produced_by(source_seq)
        .with_payload(serde_json::json!({ "reason": reason }))
    }
}

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

    /// 取走该会话队首的一批条目（至多 `supplements_max_per_drain` 条），并逐条发删除变更。
    ///
    /// 抽干与变更通知必须成对（[`announce_drained`]）：分开写就会出现"抽了没通知"
    /// （前端以为条目还在）或"通知了没抽"（条目凭空消失）两种静默不一致。
    ///
    /// `pub(crate)`：轮边界抽干（[`super::super::chat_loop::SupplementDrain`]）经它的
    /// 同步孪生 [`take_inbox_batch_sync`] 取批，再交给
    /// [`super::supplements::merge_supplements`] 合并。两个抽干点共用同一个上界判定与
    /// 同一份通知出口——各写一遍必然在"上界取谁""变更发几次"上漂移。
    pub(crate) async fn take_inbox_batch(&self, state: &Arc<ActiveSessionState>) -> Vec<InboxItem> {
        let limit = {
            let cfg = self.config.read().await;
            drain_limit(&cfg)
        };

        let batch: Vec<InboxItem> = {
            let mut inner = state.inner.write().await;
            let n = inner.inbox.len().min(limit);
            inner.inbox.drain(..n).collect()
        };

        announce_drained(&self.change_subs, &state.session_id, &batch);
        batch
    }

    /// 取队首**一条**（一轮的最小单位），并通知这一条已出队。
    ///
    /// 与 [`Self::take_inbox_batch`] 的区别只在**数量**：开轮是「一条消息 = 一轮」，
    /// 补充合并是轮末的可选加工。按批取会把合并上界之外的消息一起抽走，
    /// 而一条只够开一轮——多抽的那些就再没有开轮的机会了。
    pub(crate) async fn take_inbox_front(
        &self,
        state: &Arc<ActiveSessionState>,
    ) -> Option<InboxItem> {
        let item = {
            let mut inner = state.inner.write().await;
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
