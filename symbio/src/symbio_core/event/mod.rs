//! 事件契约 —— v2 事件溯源的共享类型层（[plan/01 §2 / §6](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 为什么是独立模块
//!
//! 施工视图的六个包（`actors` / `projections` / `store` / `adapters` / `governance`）
//! 彼此**禁止互相依赖**，唯一例外是「事件与 View 的类型定义」——那是共享契约
//! （[plan/05 §3.1](../../../../docs/plan/05-模块架构.md)，01 §13.5「契约从边上挪到中心」）。
//! 因此事件类型必须住在**中性层**：`store`（④）与 `projection`（③）都只 import 本模块，
//! 谁也拿不到谁的句柄。
//!
//! ## 冻结锚点（[plan/03 §1](../../../../docs/plan/03-演进与验证.md)）
//!
//! - **F5**：动词 5 个、能力 7 个、模式 3 个 —— 本模块冻结动词与实体的**闭集**
//!   （`Verb` / `Entity` 枚举）。新增格子 = `match` 加一臂，是数据变化不是机制变化；
//!   枚举而不是字符串，是为了让"网格之外的事件"**编译不过**（J3）。
//! - 事件**语法网格**（01 §6）：`实体 × 动词` 的格子上有若干**名字**（如
//!   `chat.assistant.final` 落在 `turn × closed`）。名字是数据，加名字不加枚举。

use serde::{Deserialize, Serialize};

/// 不可伪造的单调序号（[plan/01 §2](../../../../docs/plan/01-核心架构.md)）。
///
/// 字段私有：**只有 `Store` 能构造**（`pub(crate)`），调用方只能拿到与比较——
/// 自己 `Seq(5)` 伪造一个序号编译不过。这就是「类型强制单调」的构造侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Seq(u64);

impl Seq {
    /// 仅 crate 内（`Store` 实现）可构造。
    pub(crate) fn new(v: u64) -> Self {
        Seq(v)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

/// 事件时间戳：Unix 毫秒（UTC），口径同 [`crate::symbio_core::clock_now_ms`]。
pub type Timestamp = i64;

// ── 事件名字表（S01：turn/* 全部，[roadmap/S01 §3](../../../../docs/plan/roadmap/S01-最小闭环.md)）────
//
// 名字是**数据**（加名字不加枚举，01 §6）；但名字必须**单点定义**——散在调用点的
// 字符串字面量必然漂移（同 merge_supplements 的单点判据）。常量只钉名字，
// 网格坐标（落在哪个 entity × verb 格子）由构造方用枚举保证。

/// 用户发言。落在 `turn × opened`。
pub const EVENT_USER_MESSAGE: &str = "user.message";
/// 助手最终答复（每 turn 至多 1 条——N3）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FINAL: &str = "chat.assistant.final";
/// 兜底话术——**普通事件，不是特殊通道**（生成失败也要有输出）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";
/// 断点（S4：`store = wal` 的伴随事件，[roadmap/S05 §3](../../../../docs/plan/roadmap/S05-长会话与断点恢复.md)）。
/// 载荷携带可序列化的 checkpoint 状态。落在 `thread × progressed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑤ 接线后摘除
pub const EVENT_THREAD_CHECKPOINT: &str = "thread.checkpoint";

// ── 产物格子（S02 §3，[roadmap/S02-工具调用与产物.md](../../../../docs/plan/roadmap/S02-工具调用与产物.md)）──
//
// 工具**结果**是产物事实：模型请求了工具、工具产出了这段东西。工具调用**本身**
// 不单独占格（同一件事两格必然漂移）；`produced_by` 指向**本轮用户格**，即
// 「这个产物是本轮用户请求的后果」——S02 §3 的验收断言正是 `caused_by` 指向发起
// 它的 `task/turn`。

/// 产物产出：一次工具调用的**结果**（载荷 `{ tool, text }`）。
/// 落在 `artifact × asserted`（网格坐标见 [plan/01 §6](../../../../docs/plan/01-核心架构.md)）。
pub const EVENT_ARTIFACT_ADDED: &str = "artifact.added";

// ── 记忆格子（S5，[roadmap/S06 §3](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md)）────

/// 编码：学到的语义内容 + embedding（`payload: { content, tag, vec }`）。
/// 落在 `memory × opened`。**记忆必带溯源**（I2 扩展：溯源覆盖 100%）。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑦ 接线后摘除
pub const EVENT_MEMORY_ENCODED: &str = "memory.encoded";
/// 巩固：压缩 + 反事实（不是逐帧回放）。落在 `memory × progressed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑦ 接线后摘除
pub const EVENT_MEMORY_CONSOLIDATED: &str = "memory.consolidated";
/// 遗忘：**不是物理删除**——Log 永不删，只是投影不再包含（可撤销、可审计）。
/// 落在 `memory × closed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑦ 接线后摘除
pub const EVENT_MEMORY_FORGOTTEN: &str = "memory.forgotten";
/// 召回：检索 Translator 的产出（读视图 → 产出事实）。落在 `memory × asserted`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑦ 接线后摘除
pub const EVENT_MEMORY_RECALLED: &str = "memory.recalled";

// ── 承诺格子（S6 第 15 步，[roadmap/S08 §3](../../../../docs/plan/roadmap/S08-多主体与对等承诺.md)）──
//
// 立约 = `commitment` 实体的一格事件（**加格子，不加机制**——对等协作是图，
// 子智能体递归的树表达不了，才需要这组格子）。声誉是它的投影（`projection = reputation`）。

/// 立约：`from` 向 `to` 承诺交付什么（载荷 `{ id, from, to, promise }`）。
/// 落在 `commitment × opened`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑧ 接线后摘除
pub const EVENT_COMMITMENT_OFFERED: &str = "commitment.opened";
/// 守约收束：承诺按约履行。落在 `commitment × closed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑧ 接线后摘除
pub const EVENT_COMMITMENT_RELEASED: &str = "commitment.released";
/// 违约收束：承诺未履行（载荷带 `why`——违约必须可观测）。落在 `commitment × closed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑧ 接线后摘除
pub const EVENT_COMMITMENT_BROKEN: &str = "commitment.broken";
/// 对等声明：把承诺状态对等宣告给协作方（不是新通道，是普通事件）。
/// 落在 `commitment × asserted`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑧ 接线后摘除
pub const EVENT_COMMITMENT_ASSERTED: &str = "commitment.asserted";

// ── 任务格子（S7 第 16–18 步，[roadmap/S03 §3](../../../../docs/plan/roadmap/S03-多步任务与返工.md)）──
//
// 任务图（DAG）= 事件的 `depends_on` 载荷字段——**数据，不是机制**；
// 返工 = 新增一条事件（`task.rework_created`），不是修改历史（append-only
// 已经提供了撤销语义）。落在 `task × opened / progressed / held / asserted`。

/// 开任务：载荷 `{ task_id, depends_on, goal }`。落在 `task × opened`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑨ 接线后摘除
pub const EVENT_TASK_OPENED: &str = "task.opened";
/// 推进：执行中的普通事实。落在 `task × progressed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑨ 接线后摘除
pub const EVENT_TASK_PROGRESS: &str = "task.progress";
/// 暂挂（插话抢占 / 等外部资源；[roadmap/S07](../../../../docs/plan/roadmap/S07-插话与实时打断.md)
/// 里这一格的事件名叫 `task.blocked`，本仓库以 `task.held` 单点定义）。
/// 落在 `task × held`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑨ 接线后摘除
pub const EVENT_TASK_HELD: &str = "task.held";
/// 终态：验证通过（验收通过才终态）。落在 `task × asserted`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑨ 接线后摘除
pub const EVENT_TASK_ASSERTED: &str = "task.asserted";
/// 返工：判定不合格 ⇒ **新增一条事件**（重开一个返工节点），不是回滚。
/// 落在 `task × asserted`（返工本身是一次质量判定的事实）。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑨ 接线后摘除
pub const EVENT_TASK_REWORK_CREATED: &str = "task.rework_created";

// ── 控制格子（S8 第 19–20 步，[roadmap/S07 §3](../../../../docs/plan/roadmap/S07-插话与实时打断.md)、
//    [roadmap/S09 §3](../../../../docs/plan/roadmap/S09-外部执行与熔断.md)）────
//
// 打断信号与熔断**共用一格**：`control/opened`（kinds 都是 `task.controlled`，
// 以载荷 `reason` 区分）——插话的"停/继续/改道"与外部的"熔断"是同一类事实：
// 一个更快的判定者决定了对在跑事务的处置。不加新机制，只加一格。

/// 控制判定产出（打断处置 / 熔断）。载荷 `{ task_id?, reason, ... }`。
/// 落在 `control × opened`。**抢占判定者只持 JudgeIntent**——无 `reply.*` 写权，
/// 判定者不得直接发言（S07 §5）。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑩ 接线后摘除
pub const EVENT_CONTROL_OPENED: &str = "task.controlled";

// ── 系统与「欲」格子（S9 第 21 步，[roadmap/S12 §3](../../../../docs/plan/roadmap/S12-自主层与长期目标.md)）──
//
// 自主层**不是新架构层**：定时触发 = 触发器产出事件（不是旁路）；长目标 = 一条
// `task.opened`，`budget_ms = 86400000`（四层时延的第四个取值，同一参数）；
// 「欲」= `event.entity = conation` 的新取值（02 §2.3 的 E1）。

/// 定时触发：没有用户消息时的自主行为起点。落在 `system × opened`。
/// **触发器产出事件，不是旁路**——自主行为同样走 I1 单通道、I2 带溯源。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑪ 接线后摘除
pub const EVENT_SYSTEM_TRIGGERED: &str = "system.triggered";
/// 健康自检。落在 `system × progressed`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑪ 接线后摘除
pub const EVENT_SYSTEM_HEALTH: &str = "system.health";
/// 「欲」的表达：一条意图出现（E1：欲是数据，走 I1 单通道、I2 带溯源——
/// 无 `produced_by` 的欲事件**构造不出**候选意图，见 ② IntentGate）。
/// 落在 `conation × opened`。
#[allow(dead_code)] // dead-code-allow R-002: 事件名字表：名字是数据、单点定义（README §1.2 event 行 / ADR-043）；04 §3.1 批⑪ 接线后摘除
pub const EVENT_CONATION_EXPRESSED: &str = "conation.expressed";

/// 事件实体 —— 语法网格的**行**，闭集（F5，10 个）。
///
/// 新增实体是 L-schema 变化（[plan/03 §2](../../../../docs/plan/03-演进与验证.md)）：
/// 加枚举臂 + ADR 登记，不是机制变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Entity {
    Turn,
    Task,
    Artifact,
    Verdict,
    Memory,
    Commitment,
    Conation,
    Thread,
    Control,
    System,
}

/// 事件动词 —— 语法网格的**列**，闭集（F5，5 个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Verb {
    Opened,
    Progressed,
    Held,
    Closed,
    Asserted,
}

/// 事件信封：一 条已发生事实的全部元数据。
///
/// **载荷与元数据分两层**：`payload` 是格子自己的数据（如发言内容）；
/// 其余字段是**所有事件共有**的溯源 / 预算元数据（I2 无溯源不声明、I3 到点必答）。
///
/// `seq` 入库前为 `None`——它由 [`crate::symbio_core::store::Store::append`]
/// 在唯一写入口内**分配**，调用方无权指定（append-only + 单调的构造侧保证）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 幂等键：重复 `append` 同一 `event_id` ⇒ [`crate::symbio_core::store::AppendError::Duplicate`]。
    pub event_id: String,
    /// 网格名（数据），如 `"chat.assistant.final"`；落在 `entity × verb` 格子上。
    pub kind: String,
    pub entity: Entity,
    pub verb: Verb,
    /// 所属回合。N3（每 turn 至多一条 final）按它分桶。
    pub turn: u64,
    /// 产生者（Principal，数据：字符串 ID）。
    pub actor: String,
    /// 溯源：产生本事件的上游事件 seq（I2——断言类事件必须非空）。
    pub produced_by: Option<u64>,
    /// 产生本事件花费的毫秒数（I3 记账）。
    pub cost_ms: u64,
    /// 入库时间（Unix 毫秒）。
    pub ts: Timestamp,
    /// 格子自己的数据。
    pub payload: serde_json::Value,
    /// 由 `Store` 分配的全局单调序号；入库前 `None`。
    pub seq: Option<Seq>,
}

impl Event {
    /// 造一条**未入库**事件（seq 待 `Store` 分配）。
    pub fn pending(
        event_id: impl Into<String>,
        kind: impl Into<String>,
        entity: Entity,
        verb: Verb,
        turn: u64,
        actor: impl Into<String>,
    ) -> Self {
        Event {
            event_id: event_id.into(),
            kind: kind.into(),
            entity,
            verb,
            turn,
            actor: actor.into(),
            produced_by: None,
            cost_ms: 0,
            ts: 0,
            payload: serde_json::Value::Null,
            seq: None,
        }
    }

    /// 设置溯源（I2）——链式写法，方便构造。
    pub fn with_produced_by(mut self, seq: u64) -> Self {
        self.produced_by = Some(seq);
        self
    }

    /// 设置预算记账（I3）。
    pub fn with_cost_ms(mut self, cost_ms: u64) -> Self {
        self.cost_ms = cost_ms;
        self
    }

    /// 设置时间戳。
    pub fn with_ts(mut self, ts: Timestamp) -> Self {
        self.ts = ts;
        self
    }

    /// 设置载荷。
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }
}

/// 信封的最小契约（[plan/01 §2](../../../../docs/plan/01-核心架构.md) 的 `type Event: EventEnvelope`）。
///
/// `Store` 只需要知道三件事：幂等键是什么、seq 在不在、seq 怎么赋值。
/// 剩余字段对存储层不可见——将来 WAL / 分片实现携带更富的信封也不动 trait（F1）。
///
/// `Send + Sync` 是事件作为**纯数据**的本性（快照要跨线程交给投影），
/// 放在超 trait 上，任何实现都无法造出不能跨线程的事件。
pub trait EventEnvelope: Clone + Send + Sync {
    /// 幂等键。
    fn event_id(&self) -> &str;
    /// 入库前 `None`。
    fn seq(&self) -> Option<Seq>;
    /// **仅由 `Store` 在唯一写入口内调用。**
    fn assign_seq(&mut self, seq: Seq);
}

impl EventEnvelope for Event {
    fn event_id(&self) -> &str {
        &self.event_id
    }

    fn seq(&self) -> Option<Seq> {
        self.seq
    }

    fn assign_seq(&mut self, seq: Seq) {
        self.seq = Some(seq);
    }
}
