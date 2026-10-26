//! 调度者（S04 §3 的 `Decider` 行）：就绪集 → 发放 / 到期判死。
//!
//! ## 它是一个 `Decider`，不是组件
//!
//! [DE-034](../../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约") §2 把「调度器」拆成
//! 「一个 `Decider`，输入 `ReadySet` 视图，输出 `task.assigned` 事件」。它的判据是**规则**
//! （确定性、毫秒级），因此模式是 [`Pattern::Decider`]；它不调模型、不持有任何句柄，
//! 只把一张视图算成一批事实。
//!
//! ## 三件事，按这个顺序
//!
//! 1. **判死到期的租约**：`task.assigned` 的 `until_ms <= now` 且任务非终态 ⇒ 补一条
//!    `task.ready{reason: "lease_expired"}`。持有状态由投影算（`readyset` 只认时间戳），
//!    但「那笔租约死了」这件事**要留下事实**——否则回放里它只是「突然又能派了」。
//! 2. **记下这次算过就绪**：候选集里每一笔落 `task.ready`（同一开启周期内只落一次）。
//!    只落输出（`assigned`）分不开「算过但没派」与「压根没算」（`D-139` 裁决 1）。
//! 3. **发放**：按 `(opened_seq, task_id)` 升序，给候选集的每一笔落
//!    `task.assigned{owner, lease_ms, until_ms}`。候选集已被 `cap` 截断——`cap = 1`
//!    因此就是「一次派一笔」的串行 FIFO。
//!
//! ## 写侧闸：`assign.work`
//!
//! 落 `task.assigned` 需要资格。[`crate::symbio_core::actor_can_assign_work`] 读的是**规格自己身上**的
//! 能力集（构造时来自授权表行），与收束两臂 / 判意图 / 盖章四臂同一条纪律：判的对象是
//! 规格，构造的来源是表行，单一链条。缺该能力 ⇒ **一条事实都不写**（fail-closed）——
//! 没有资格的调度不该留下「我派过」的痕迹。
//!
//! ## 它不做什么
//!
//! 不做优先级与公平性（`readyset` 的打分参数）、不做抢占（那是 `PreemptionDecider`
//! 的活）、不做验收（`task.asserted` 归验证者）。它只回答一个问题：**这一刻该把哪几笔
//! 活交出去，交给谁，租到什么时候**。

use symbio_kernel::actors::{ActorSpec, Pattern, Scope};

use crate::symbio_core::authz::PRINCIPAL_SCHEDULER;
use crate::symbio_core::projection::readyset::ReadySetView;
use crate::symbio_core::{
    Event, EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED, EVENT_TASK_HELD, EVENT_TASK_OPENED,
    EVENT_TASK_READY,
};

use std::collections::BTreeMap;

/// 调度者的时延预算：反射档（`LatencyTier::Reflex`）——它是一次纯计算 + 几次落格。
pub const SCHEDULE_BUDGET_MS: u64 = 80;

/// 一笔租约的时长（ms）。
///
/// 取值理由与端口租约同一条：**比最慢的可接受持有长，比「用户已经以为坏了」短**。
/// 任务是模型驱动的一步工作，分钟级足够；到期不续即放回候选池（不是重试——重试语义
/// 归端口，见 `plugins/loop/port`）。
pub const LEASE_MS: i64 = 300_000;

/// 调度者要落的一笔事实（**只描述，不构造 `Event`**）。
///
/// 事件信封（`event_id` / `turn` / `produced_by` / 溯源）由写方
/// （`plugins/session/task_events` 的转写口）统一给——调度者只回答「该有什么」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleFact {
    /// 事件种类：`task.ready` 或 `task.assigned`。
    pub kind: &'static str,
    /// 载荷（含 `task_id`——任务格的**身份字段**，每一格都必须带，投影按它归户）。
    pub payload: serde_json::Value,
}

/// 调度者。
pub struct ScheduleDecider;

impl ScheduleDecider {
    /// **生产构造点**：调度者的 `ActorSpec`（[`Pattern::Decider`] / 80ms / 根作用域）。
    ///
    /// 能力集从**授权表的行**取（`grant_names_of`），不在这里列名字——与
    /// [`trivial_actor_spec`](crate::symbio_core::trivial_actor_spec) 同一条口径：「规格说得出自己有什么权」，读的是登记事实。
    pub fn spec() -> ActorSpec {
        ActorSpec {
            capabilities: crate::symbio_core::authz::production_matrix()
                .grant_names_of(PRINCIPAL_SCHEDULER),
            pattern: Pattern::Decider,
            budget_ms: SCHEDULE_BUDGET_MS,
            scope: Scope::Root,
            principal: PRINCIPAL_SCHEDULER.to_string(),
        }
    }

    /// 算一次调度：给 `view` 里的候选集**记就绪 + 发放租约**，并判死已到期的租约。
    ///
    /// - `events`：完整史（判「这笔租约是不是已经死过一次」「这一次就绪记过没有」）；
    /// - `owner`：租约持有者（当前执行主体——调度者只派活，不假装自己干活）；
    /// - `now`：当前时刻（ms）。
    ///
    /// 返回按**落格顺序**排好的事实：先判死、再记就绪、最后发放。
    pub fn decide(
        &self,
        spec: &ActorSpec,
        view: &ReadySetView,
        events: &[Event],
        owner: &str,
        now: i64,
    ) -> Vec<ScheduleFact> {
        // 写侧闸：没有 `assign.work` ⇒ 什么都不写（fail-closed）。
        if !crate::symbio_core::actor_can_assign_work(spec) {
            return Vec::new();
        }
        let mut out = Vec::new();

        // ── 1. 判死到期的租约 ────────────────────────────────────────────────
        for (task_id, expired) in expired_leases(events, now) {
            out.push(ScheduleFact {
                kind: EVENT_TASK_READY,
                payload: serde_json::json!({
                    "task_id": task_id,
                    "reason": "lease_expired",
                    "expired_owner": expired.owner,
                    "until_ms": expired.until_ms,
                }),
            });
        }

        // ── 2 / 3. 记就绪 + 发放 ─────────────────────────────────────────────
        for task in &view.ready {
            if !ready_recorded(events, &task.task_id) {
                out.push(ScheduleFact {
                    kind: EVENT_TASK_READY,
                    payload: serde_json::json!({
                        "task_id": task.task_id,
                        "opened_seq": task.seq,
                    }),
                });
            }
            out.push(ScheduleFact {
                kind: EVENT_TASK_ASSIGNED,
                payload: serde_json::json!({
                    "task_id": task.task_id,
                    "owner": owner,
                    "lease_ms": LEASE_MS,
                    "until_ms": now + LEASE_MS,
                }),
            });
        }
        out
    }
}

/// 一笔已到期的租约（`until_ms <= now` 且任务未终态、判死还没记过）。
struct Expired {
    owner: String,
    until_ms: i64,
}

/// 扫一遍史，找出**该判死**的租约：最新一条「发放」晚于最新一条「就绪或终态」，
/// 且它的 `until_ms` 已经过去。
///
/// 判据只用 seq 与时间戳（不含任何内存状态）：同一份切片永远算出同一批结果。
fn expired_leases(events: &[Event], now: i64) -> Vec<(String, Expired)> {
    // task_id → (最新 assigned 的 seq, owner, until_ms)
    let mut latest_assign: BTreeMap<&str, (u64, &str, i64)> = BTreeMap::new();
    // task_id → 最新「把这笔租约了掉」的 seq（`task.ready` 的补记 / 终态 / 挂起）
    let mut settled: BTreeMap<&str, u64> = BTreeMap::new();
    for e in events.iter().filter(|e| e.ts <= now) {
        if e.entity != crate::symbio_core::Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
        match e.kind.as_str() {
            EVENT_TASK_ASSIGNED => {
                let owner = e
                    .payload
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let until_ms = e
                    .payload
                    .get("until_ms")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(i64::MIN);
                latest_assign.insert(id, (seq, owner, until_ms));
            }
            EVENT_TASK_READY | EVENT_TASK_ASSERTED | EVENT_TASK_HELD => {
                settled.insert(id, seq);
            }
            _ => {}
        }
    }
    let mut out: Vec<(String, Expired)> = latest_assign
        .into_iter()
        .filter(|(id, (seq, _, until_ms))| {
            let alive = settled.get(*id).is_none_or(|s| s < seq);
            alive && *until_ms <= now
        })
        .map(|(id, (_, owner, until_ms))| {
            (
                id.to_string(),
                Expired {
                    owner: owner.to_string(),
                    until_ms,
                },
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 这笔任务的**本轮就绪周期**里记过就绪没有：最新一条 `task.ready` 晚于最新一条
/// `task.opened`（返工会开新节点 ⇒ 新 id，旧的就绪记录不沾新节点）。
fn ready_recorded(events: &[Event], task_id: &str) -> bool {
    let mut opened: Option<u64> = None;
    let mut ready: Option<u64> = None;
    for e in events {
        if e.entity != crate::symbio_core::Entity::Task {
            continue;
        }
        if e.payload.get("task_id").and_then(|v| v.as_str()) != Some(task_id) {
            continue;
        }
        let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
        match e.kind.as_str() {
            EVENT_TASK_OPENED => opened = Some(seq),
            EVENT_TASK_READY => ready = Some(seq),
            _ => {}
        }
    }
    match (opened, ready) {
        (Some(o), Some(r)) => r > o,
        _ => false,
    }
}
