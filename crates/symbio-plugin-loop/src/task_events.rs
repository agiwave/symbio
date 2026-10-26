//! 任务表写方（S7 步 16–18，[04 §3.1 批⑨](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
//! **会话任务清单 → v2 事实源 → 两个消费方**（读出口 `session/stats` 的 `readyset`
//! 列 + 交给执行者的调度段）。
//!
//! 与 `super::memory_events` 同族：任务不是另开一份存储，任务就是
//! `v2-events.wal` 里的 `task.*` 事件（D-044 同族纪律）。清单本身（
//! `local/todo_write`）是**纯内存、会话作用域**的（见其模块头）；事实源是同一份
//! 状态的持久化，两处各算一次就是两套真相——所以本模块只**转写观察到的声明**，
//! 不自己维护任务图。
//!
//! ## 两处消费方（一个事实一个出口）
//!
//! | 消费方 | 形态 | 门控 |
//! |---|---|---|
//! | 读出口 `session/stats` | `readyset` 列（与四列同款：事实源读什么算什么） | `may_read` |
//! | 调度段 [`prompt_section`] | 请求视图**置顶**一段 `role=user` 的提示 | 有任务 ∧ 就绪集非空 ∧ `request_round == 0` |
//!
//! 投影给的是**候选集**（`readyset` 是纯函数，同一份切片永远算出同一个集合），
//! 选哪一个是调用方的策略——调度段把候选集交给本轮的执行者（模型），不替它决定
//! （`projection::readyset` 模块头的边界：优先级 / 公平性 / 饥饿都是打分参数）。
//!
//! ## 六条口径（为什么是这样）
//!
//! - **只记声明，不记全量**：`merge = true` 是增量，`merge = false` 是整体替换——
//!   本模块两种模式**行为相同**，因为事实源只认被声明到的项。清单里没提到的任务
//!   一律不动。
//! - **整体替换移出清单的项保持未终态**：append-only 不替模型编一条「已验收」的
//!   假事实。代价写在明处——它仍留在事实源里、也就仍可能留在就绪集里，读出口会
//!   念出一个清单上已经没有的 id。要表达「不再做」得先有一格 cancel 语义的事件，
//!   而 `task.*` 词汇表里没有这一格 ⇒ 本模块不替模型决定那个 id 该算什么。
//! - **降级不产事实**（进行中 → 未开始）：`task.progress` 是不可撤销的推进记录，
//!   清单把它改回 `pending` 只是模型改了主意，没有新事实发生。既不写事件、也不改
//!   观察状态，跨轮因此稳定（不会每次声明都补一条）。
//! - **回退出终态 = 返工**：`completed → pending / in_progress` 写
//!   `task.rework_created` + 一个**新节点**的 `task.opened`（`{id}-r{n}`），
//!   旧节点仍是终态——返工是新增一条事实，不是回滚
//!   （[roadmap/S03 §2](../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")）。
//!   这正是 `rework_bounded` 的**生产写方**：不是一条永远空转的断言。
//! - **溯源锚**：全部锚在本轮 `user.message` 格（`user_seq`）——任务是「这轮模型
//!   说该做什么」，出处是这一轮的开口；`task.asserted` 是断言类事件，无溯源即违规
//!   （I2）。事件号 `{id_prefix}-{n}`：`id_prefix` 由调用方给
//!   （[`super::derived_facts::record_derived`] 给 `v2t-{anchor_id}`，一轮一个前缀）
//!   + 本次序号，跨轮不撞幂等键。
//! - **写方**：写方住收束处——`turn_exec` 轮末调
//!   [`super::derived_facts::record_derived`]，后者调本模块（同一锚 `user_seq`）。
//!   别按「与记忆同一个写方」推断——记忆与学习住
//!   [`super::derived_facts::record_learning`]，承诺 / 任务表 / 熔断住
//!   [`super::derived_facts::record_derived`]；写侧授权闸判在运行器
//!   （见 `turn_exec` 模块文档的「写侧授权闸」）。
//!
//! ## 诚实划界
//!
//! - **悬空依赖是断言的本职**：模型声明了一个不存在的依赖，`readyset` 会诚实地
//!   认为它不就绪，`acyclic_deps` 会在 `invariants` 列报出来。`depends_on` 默认
//!   `[]`，日常调用不踩这条线。
//! - **返工节点继承被返工任务的依赖**（返工做的是同一件事，前置条件不变）；
//!   而**别的任务**对旧 id 的依赖看的是旧节点的终态——旧节点验收过一次是事实，
//!   返工不追溯推翻它（要追溯得有「撤销终态」这一格，append-only 没有）。

use std::collections::BTreeMap;

use symbio_kernel::actors::{ActorSpec, Pattern, Scope};

use symbio_core::ReadySetView;
use symbio_core::PRINCIPAL_SCHEDULER;
use symbio_core::{
    actor_can_assign_work, Entity, Event, Store, Verb, EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED,
    EVENT_TASK_HELD, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS, EVENT_TASK_READY,
    EVENT_TASK_REWORK_CREATED,
};

// 记录类型住 core 词汇表（[D-082](../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 5 组）：
// 声明的写方在工具执行层，本文件是读侧投影。
use symbio_core::facts::{TaskDeclaration, TaskItem, TaskStatus};

/// 一次要补进事实源的事件（算完再写：状态推进与写入分开，便于单测断言「该补几条」）。
struct Emitted {
    kind: &'static str,
    verb: Verb,
    payload: serde_json::Value,
}

/// 清单 id → 观察状态（**从事件序列重建**，不跨轮持有内存记账——同一份切片
/// 必须永远算出同一套状态，否则单测与生产会各走一套）。
#[derive(Debug, Default, Clone)]
struct Entry {
    /// 该清单 id 当前对应的**存活节点**（返工后是 `{id}-r{n}`）。
    live: String,
    /// 存活节点的状态。
    status: TaskStatus,
    /// 存活节点的依赖（返工节点继承它）。
    depends_on: Vec<String>,
    /// 存活节点的任务内容。
    goal: String,
    /// 已发生的返工轮数。
    rounds: u32,
}

/// 载荷里的字符串字段（缺省 = 空串 / 空表，与 `readyset` 的读法同口径）。
fn payload_str(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn payload_deps(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 从已有事实重建观察状态。
///
/// 返工节点用 `task.rework_created` 载荷的 `replaces` 归到其清单项上：只有那一格
/// 携带 `replaces`，所以先登记「节点 id → 清单 id」，此后落在返工节点上的
/// opened / progressed / asserted 都按这张表归位——否则返工会在表里开出第二个条目，
/// 同一个清单项被记成两件事。
fn rebuild(snapshot: &[Event]) -> BTreeMap<String, Entry> {
    let mut map: BTreeMap<String, Entry> = BTreeMap::new();
    let mut nodes: BTreeMap<String, String> = BTreeMap::new();
    for e in snapshot {
        if e.entity != Entity::Task {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if e.kind == EVENT_TASK_REWORK_CREATED {
            let replaces = payload_str(&e.payload, "replaces");
            if replaces.is_empty() {
                continue;
            }
            nodes.insert(id.to_string(), replaces.clone());
            let round = e
                .payload
                .get("round")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1) as u32;
            let entry = map.entry(replaces).or_default();
            entry.rounds = entry.rounds.max(round);
            continue;
        }
        let key = nodes.get(id).map(String::as_str).unwrap_or(id);
        match e.kind.as_str() {
            k if k == EVENT_TASK_OPENED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Pending;
                entry.depends_on = payload_deps(&e.payload, "depends_on");
                entry.goal = payload_str(&e.payload, "goal");
            }
            k if k == EVENT_TASK_PROGRESS => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::InProgress;
            }
            k if k == EVENT_TASK_ASSERTED => {
                let entry = map.entry(key.to_string()).or_default();
                entry.live = id.to_string();
                entry.status = TaskStatus::Completed;
            }
            _ => {}
        }
    }
    map
}

/// 一次声明该补的事件（状态推进的**判定方**：只看观察状态 + 声明态，不看清单全量）。
fn step(entry: &mut Entry, list_id: &str, item: &TaskItem) -> Vec<Emitted> {
    let mut out = Vec::new();
    if entry.live.is_empty() {
        // 第一次见到这个清单 id：先开格（`readyset` 只在 `task.opened` 建节点，
        // 没有这一格的任务根本不在调度的候选集里）。
        entry.live = list_id.to_string();
        entry.status = TaskStatus::Pending;
        entry.depends_on = item.depends_on.clone();
        entry.goal = item.goal.clone();
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": list_id,
                "depends_on": item.depends_on,
                "goal": item.goal,
            }),
        });
    }

    // 降级（进行中 → 未开始）不产事实：见模块头「降级不产事实」。
    if item.status == TaskStatus::Pending {
        return out;
    }

    if entry.status == TaskStatus::Completed && item.status != TaskStatus::Completed {
        // 回退出终态 = 返工：新增一条判定事实 + 一个新节点，旧节点仍是终态。
        entry.rounds += 1;
        let node = format!("{list_id}-r{}", entry.rounds);
        out.push(Emitted {
            kind: EVENT_TASK_REWORK_CREATED,
            verb: Verb::Asserted,
            payload: serde_json::json!({
                "task_id": node,
                "replaces": list_id,
                "round": entry.rounds,
            }),
        });
        out.push(Emitted {
            kind: EVENT_TASK_OPENED,
            verb: Verb::Opened,
            payload: serde_json::json!({
                "task_id": node,
                "depends_on": entry.depends_on,
                "goal": entry.goal,
            }),
        });
        entry.live = node;
        entry.status = TaskStatus::Pending;
    }

    if entry.status != item.status {
        match item.status {
            TaskStatus::InProgress => {
                out.push(Emitted {
                    kind: EVENT_TASK_PROGRESS,
                    verb: Verb::Progressed,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::InProgress;
            }
            TaskStatus::Completed => {
                out.push(Emitted {
                    kind: EVENT_TASK_ASSERTED,
                    verb: Verb::Asserted,
                    payload: serde_json::json!({ "task_id": entry.live }),
                });
                entry.status = TaskStatus::Completed;
            }
            // `Pending` 已在上面提前返回。
            TaskStatus::Pending => {}
        }
    }
    out
}

/// 把本轮的 `todo_write` 声明写进事实源（任务表写方，批⑨；由
/// [`super::derived_facts::record_derived`] 轮末调用）。
///
/// - `produced_by` = 本轮 `user.message` 的 seq（溯源锚，I2）；
/// - `id_prefix` = `v2t-t{turn}`，事件号是 `{id_prefix}-{n}`；
/// - 失败冒泡给调用方记日志（与记忆三段同款：附加事实失败不算收束失败）。
pub(crate) fn write(
    store: &dyn Store<Event = Event>,
    snapshot: &[Event],
    decls: &[TaskDeclaration],
    turn: u64,
    produced_by: u64,
    principal: &str,
    id_prefix: &str,
) -> Result<(), String> {
    if decls.is_empty() {
        return Ok(());
    }
    let mut map = rebuild(snapshot);
    let mut n = 0usize;
    for decl in decls {
        for item in &decl.items {
            for e in step(map.entry(item.id.clone()).or_default(), &item.id, item) {
                let event_id = format!("{id_prefix}-{n}");
                n += 1;
                store
                    .append(
                        Event::pending(event_id, e.kind, Entity::Task, e.verb, turn, principal)
                            .with_produced_by(produced_by)
                            .with_payload(e.payload),
                    )
                    .map_err(|err| format!("任务事件入格失败：{err:?}"))?;
            }
        }
    }
    Ok(())
}

/// 调度（S04 §3，`D-139` 裁决 3）：就绪集 → `task.ready` / `task.assigned`。
///
/// ## 谁写、凭什么写
///
/// 写方是**调度者**（`actors::schedule::ScheduleDecider`），不是本模块：本模块只做
/// 「把事实交给 store」这一件与其余任务事实相同的事。资格由写侧闸判
/// （`ActorSpec::can_assign_work`，缺 `assign.work` ⇒ 零事实，fail-closed）。
///
/// ## 为什么在轮末调
///
/// 调度的输入是**本轮刚写完的任务表**：模型在这一轮声明了任务（`task.opened`），
/// 轮末才有东西可派。调用点排在 [`write`] 之后、且**重新取一次史**——拿旧切片算，
/// 这一轮新开的格不在候选集里（`derived_facts::record_schedule` 的调用点注释）。
///
/// ## 事件号
///
/// `{id_prefix}-s{n}`：`id_prefix` 由调用方给（`t{turn}-s`），一次调度一个序列。
/// 与任务表写方的 `v2t-{anchor}` 前缀不同源，因此同一轮内两批事实的幂等键不撞。
#[allow(clippy::too_many_arguments)]
pub(crate) fn schedule(
    store: &dyn Store<Event = Event>,
    snapshot: &[Event],
    cap: usize,
    owner: &str,
    turn: u64,
    produced_by: u64,
    id_prefix: &str,
    now: i64,
) -> Result<(), String> {
    let spec = ScheduleDecider::spec();
    let started = symbio_core::clock_now_ms();
    let view = symbio_core::readyset(cap).read_all(snapshot).value;
    let facts = ScheduleDecider.decide(&spec, &view, snapshot, owner, now);
    // 预算（`I3` 的反射档）：**算完再判**——能拦的只有落格这一半。超预算就一笔都
    // 不落，而不是落一半：半批租约没人认账（下一轮它们既不在候选集里、也没人持有）。
    let elapsed = symbio_core::clock_now_ms() - started;
    if elapsed > SCHEDULE_BUDGET_MS as i64 {
        symbio_core::plugin_warn!(
            "session",
            "[task] 调度超预算（{elapsed}ms > {SCHEDULE_BUDGET_MS}ms）⇒ 本轮不派活"
        );
        return Ok(());
    }
    for (n, fact) in facts.into_iter().enumerate() {
        let verb = if fact.kind == symbio_core::EVENT_TASK_READY {
            Verb::Progressed
        } else {
            Verb::Asserted
        };
        store
            .append(
                Event::pending(
                    format!("{id_prefix}-{n}"),
                    fact.kind,
                    Entity::Task,
                    verb,
                    turn,
                    &spec.principal,
                )
                .with_produced_by(produced_by)
                .with_payload(fact.payload),
            )
            .map_err(|err| format!("调度事实入格失败：{err:?}"))?;
    }
    Ok(())
}

/// 调度段（S7 步 17）：就绪任务集渲染成一段提示，请求视图**置顶**注入。
///
/// 只读不写（与 `memory_events::recall_view` 同款的只读视图），因此：
/// - 没有事实源 / 读不到 ⇒ `None`——请求里就少一段，不存在「空占位」；
/// - 就绪集空 ⇒ `None`——依赖没闭合时给模型一个空的候选集只会诱导它瞎猜。
///
/// `snapshot` 是本会话事实的**值**（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：
/// 读方不收路径、不开文件——枚举与读完在存储层，这里只投影。
///
/// 调用方（`chat_loop::inputs`）按 `request_round == 0` 取一次、本轮各工具轮复用。
pub(crate) fn prompt_section(snapshot: &[Event], cap: usize) -> Option<String> {
    let ready = symbio_core::readyset(cap).read_all(snapshot).value.ready;
    if ready.is_empty() {
        return None;
    }
    let ids = ready
        .iter()
        .map(|t| t.task_id.as_str())
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "【任务调度】就绪任务（依赖已全部完成）：{ids}。请从就绪任务中推进一项；\
         其余任务的依赖尚未完成，暂不可开始。"
    ))
}

// ==================== 调度策略（D-189：唯一生产消费方在本文件） ====================

/// 调度者的时延预算：反射档（`LatencyTier::Reflex`）——它是一次纯计算 + 几次落格。
pub(crate) const SCHEDULE_BUDGET_MS: u64 = 80;

/// 一笔租约的时长（ms）。
///
/// 取值理由与端口租约同一条：**比最慢的可接受持有长，比「用户已经以为坏了」短**。
/// 任务是模型驱动的一步工作，分钟级足够；到期不续即放回候选池（不是重试——重试语义
/// 归端口，见 `plugins/loop/port`）。
const LEASE_MS: i64 = 300_000;

/// 调度者要落的一笔事实（**只描述，不构造 `Event`**）。
///
/// 事件信封（`event_id` / `turn` / `produced_by` / 溯源）由本文件的 [`schedule`] 统一给
/// ——调度者只回答「该有什么」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleFact {
    /// 事件种类：`task.ready` 或 `task.assigned`。
    pub kind: &'static str,
    /// 载荷（含 `task_id`——任务格的**身份字段**，每一格都必须带，投影按它归户）。
    pub payload: serde_json::Value,
}

/// 调度者（S04 §3 的 `Decider` 行，[DE-034](../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约")）：
/// 就绪集 → 发放 / 到期判死。
///
/// 它是一个 `Decider`，不是组件：输入 `ReadySet` 视图，输出 `task.assigned` 事件；
/// 判据是规则（确定性、毫秒级），不调模型、不持有任何句柄。
///
/// ## 三件事，按这个顺序
///
/// 1. **判死到期的租约**：`task.assigned` 的 `until_ms <= now` 且任务非终态 ⇒ 补一条
///    `task.ready{reason: "lease_expired"}`。持有状态由投影算（`readyset` 只认时间戳），
///    但「那笔租约死了」这件事**要留下事实**——否则回放里它只是「突然又能派了」。
/// 2. **记下这次算过就绪**：候选集里每一笔落 `task.ready`（同一开启周期内只落一次）。
/// 3. **发放**：按 `(opened_seq, task_id)` 升序，给候选集的每一笔落
///    `task.assigned{owner, lease_ms, until_ms}`。
///
/// ## 写侧闸：`assign.work`
///
/// 落 `task.assigned` 需要资格。[`actor_can_assign_work`] 读的是**规格自己身上**的
/// 能力集（构造时来自授权表行）。缺该能力 ⇒ **一条事实都不写**（fail-closed）。
///
/// ## 它不做什么
///
/// 不做优先级与公平性（`readyset` 的打分参数）、不做抢占（那是
/// `plugins/session/transcript/inbox.rs::PreemptionDecider` 的活）、不做验收
/// （`task.asserted` 归验证者）。它只回答一个问题：**这一刻该把哪几笔活交出去，
/// 交给谁，租到什么时候**。
pub(crate) struct ScheduleDecider;

impl ScheduleDecider {
    /// 生产构造点：调度者的 `ActorSpec`（[`Pattern::Decider`] / 80ms / 根作用域）。
    ///
    /// 能力集从**授权表的行**取（`grant_names_of`），不在这里列名字。
    pub(crate) fn spec() -> ActorSpec {
        ActorSpec {
            capabilities: symbio_core::production_matrix().grant_names_of(PRINCIPAL_SCHEDULER),
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
    pub(crate) fn decide(
        &self,
        spec: &ActorSpec,
        view: &ReadySetView,
        events: &[Event],
        owner: &str,
        now: i64,
    ) -> Vec<ScheduleFact> {
        // 写侧闸：没有 `assign.work` ⇒ 什么都不写（fail-closed）。
        if !actor_can_assign_work(spec) {
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
        if e.entity != Entity::Task {
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
        if e.entity != Entity::Task {
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

#[cfg(test)]
#[path = "task_events.test.rs"]
mod tests;
