//! 事件名字表 —— **名字 → 格位（实体 × 动词）** 收成一份数据，判它的位置在唯一写入口。
//!
//! ## 它补的是哪一格
//!
//! 网格的**形状**由两个闭集枚举钉死（[`Entity`] 11 行 × [`Verb`] 5 列，F5），
//! 名字却是**字符串常量**（[plan/01 §6](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")：
//! 「名字是数据，加名字不加枚举」）。于是「`user.message` 落在 `turn × opened`」这条
//! **对应关系**此前只有两个载体：常量头上的散文说明，与各构造点手填的 `entity` / `verb`。
//! 散文不判、构造点没人核 ⇒ 漂了没有任何信号。而投影是**按格子**取事实的
//! （[`event_node_of`](super::event_node_of) 的 `match` 三目键就是 `(entity, verb, kind)`），
//! 漂一寸的表现是**静默少一条事实**——不是报错。
//!
//! 本模块把这条关系落成 [`CELLS`] 一张表，判据是 [`event_cell`]，强制点在
//! **唯一写入口**内（会话事实源的写侧闸门，报错形态见
//! [`AppendError::OffGrid`](crate::symbio_core::AppendError::OffGrid)）。
//!
//! ## 为什么是「表 + 入库闸」，不是 `EventKind` 枚举
//!
//! 审计稿（[T-206](../../../../docs/009-task/206-事件总线读侧与协议收敛批.md "T-206-事件总线读侧与协议收敛批")
//! 来源的 08 报告建议 7）提的是「`kind` 由 `(entity, verb)` 派生，`EventKind` 枚举一张表」。
//! 那条路走不通，两条理由都是硬的：
//!
//! - **F5 与 01 §6**：加了 `EventKind` 枚举，「加一个名字」就变成「改 core 的枚举」=
//!   L-schema 变化（要 ADR），而网格的设计意图恰恰是加名字不动机制。
//! - **线上形状**：`kind` 在帧、在 WAL 行、在前端与 `cli` 里都是字符串，
//!   改成枚举要么在边界上多一次转换，要么动协议——两条都不是本批的范围。
//!
//! J3「网格之外的事件编译不过」钉的是**实体与动词**这一半（字符串写不出第三个格子，
//! 编译期已经在管）。名字这一维本来就是数据，能给它的最强约束是**入库时对齐**——
//! 与 `ts = 0` 那条同一条纪律：逐构造点手收必然再漏，故收口在入库处（见
//! [`EventEnvelope::fallback_timestamp`](crate::symbio_core::EventEnvelope::fallback_timestamp) 的教训记录）。
//!
//! ## 覆盖是自迫的，不需要新增扫描
//!
//! 表上**没有**的名字写不进事实源：闸门答 `OffGrid { declared: None, .. }`。
//! 所以「加名字的人忘了登记」这件事不会静默发生——他第一次落格就红，
//! 红得有名有姓（事件 id + 名字 + 两边格位）。无需为此再加一条门禁脚本。
//!
//! ## 表的口径
//!
//! 一行 = 一个名字。同一格可以有几个名字（`turn × asserted` 上就不止一个），
//! 一个名字只有一格。[`Entity::Thread`] 至今没有名字（F5 的 11 行里唯一的空格子），
//! 空格是网格**余量**，不是缺口——`plan/01 §6` 明说这张表「是语法的闭集，不是事件的闭集」。

use super::{
    Entity, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS, EVENT_ASSISTANT_REPORTED,
    EVENT_AUTHZ_DENIED, EVENT_AUTHZ_GRANTED, EVENT_COMMITMENT_ASSERTED, EVENT_COMMITMENT_BROKEN,
    EVENT_COMMITMENT_OFFERED, EVENT_COMMITMENT_RELEASED, EVENT_CONATION_EXPRESSED,
    EVENT_CONTROL_OPENED, EVENT_FAILED, EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_CONSOLIDATED,
    EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED, EVENT_MESSAGE_ADDED,
    EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY,
    EVENT_SENT, EVENT_SESSION_CONFIG, EVENT_SESSION_CONSOLIDATED, EVENT_SYSTEM_HEALTH,
    EVENT_SYSTEM_TRIGGERED, EVENT_TASK_ASSERTED, EVENT_TASK_HELD, EVENT_TASK_OPENED,
    EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED, EVENT_TURN_DISPATCH, EVENT_TURN_STAGE,
    EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE, EVENT_VERDICT_ASSERTED,
};

/// 网格上的一个格子：实体（行）× 动词（列）。
pub type Cell = (Entity, Verb);

/// 名字 → 格位。加名字 = 加一行（数据变化，F5）；段序与
/// [`super`] 的名字表一致，一族一段。
const CELLS: &[(&str, Cell)] = &[
    // ── 对话面（turn）──
    (EVENT_USER_MESSAGE, (Entity::Turn, Verb::Opened)),
    // 配置与派发档都是「这一轮以什么方式运行」的断言（T-225），与阶段事实同格。
    (EVENT_SESSION_CONFIG, (Entity::Turn, Verb::Asserted)),
    (EVENT_TURN_DISPATCH, (Entity::Turn, Verb::Asserted)),
    (EVENT_TURN_SUPPLEMENTED, (Entity::Turn, Verb::Asserted)),
    (EVENT_TURN_STAGE, (Entity::Turn, Verb::Asserted)),
    (EVENT_ASSISTANT_REPORTED, (Entity::Turn, Verb::Asserted)),
    (EVENT_ASSISTANT_OPENING, (Entity::Turn, Verb::Asserted)),
    (EVENT_ASSISTANT_FINAL, (Entity::Turn, Verb::Closed)),
    (EVENT_ASSISTANT_FALLBACK, (Entity::Turn, Verb::Closed)),
    (EVENT_ASSISTANT_PROGRESS, (Entity::Turn, Verb::Progressed)),
    // ── 协议族（对外调用，T-224）─────────────────────────────────────────
    //
    // 五个名字**同格同族**：`turn × asserted` = 「关于这一轮的断言」这一格的成员。
    // 逐条理由（为什么不是 `progressed` / `closed` / `opened`）：
    //
    // - `request`（该调了）：一条关于这一轮的断言，不是状态迁移。`opened` 是轮次
    //   的**开始**（一个 turn 只能有一次），协议事实不是。
    // - `sent`（已经派出去了）：与 `request` 同族——一次调用的两个阶段是同一条账的
    //   两个断言。**由引擎在 `before` 位写**（搬运者可能不产事实就返回，依赖它自觉
    //   ⇒ 派发行永远不还清且无人报错）。
    // - `response`（回来了）：**不是收束格**——轮子由 `chat.assistant.final` /
    //   `fallback` 收束，一次端口应答不结束一轮。
    // - `failed`（没成功）：**不是收束格**——失败的一轮仍可能重试 / 降级 / 兜底。
    //   与 `authz.denied` 同一条纪律：把它算成收束会掩盖「静默中断」。
    // - `retry`（这次尝试作废）：租约看护者落的一次尝试级否定断言。
    //
    // **为什么不落 `turn × progressed`**：那一格是**用户看得见**的动静
    // （`chat.assistant.progress`），协议事实是机器对机器的账；且 `needs_provenance`
    // 只对 `progressed` 里的 `chat.assistant.progress` 要求溯源——协议事实落那一格
    // 就会**静默失去 I2 的溯源要求**（断言类才有）。落 `asserted` ⇒ 五条都自动受
    // 「断言类必须带 `produced_by`」约束。
    (EVENT_REQUEST, (Entity::Turn, Verb::Asserted)),
    (EVENT_SENT, (Entity::Turn, Verb::Asserted)),
    (EVENT_RESPONSE, (Entity::Turn, Verb::Asserted)),
    (EVENT_FAILED, (Entity::Turn, Verb::Asserted)),
    (EVENT_RETRY, (Entity::Turn, Verb::Asserted)),
    // 否定事实与收束格**不同格**（`asserted` vs `closed`）：被拒的一轮仍然未收束。
    (EVENT_AUTHZ_DENIED, (Entity::Turn, Verb::Asserted)),
    // 放行事实与否定事实同格：都是「关于这一次授权判定的断言」，不是收束。
    (EVENT_AUTHZ_GRANTED, (Entity::Turn, Verb::Asserted)),
    // ── 产物（artifact）──
    (EVENT_ARTIFACT_ADDED, (Entity::Artifact, Verb::Asserted)),
    // ── 消息列表的变更（message，ADR-053 / ADR-054 的闭集四格）──
    (EVENT_MESSAGE_ADDED, (Entity::Message, Verb::Opened)),
    (EVENT_MESSAGE_EDITED, (Entity::Message, Verb::Asserted)),
    (EVENT_MESSAGE_REMOVED, (Entity::Message, Verb::Closed)),
    (
        EVENT_SESSION_CONSOLIDATED,
        (Entity::Message, Verb::Progressed),
    ),
    // ── 记忆（memory）──
    (EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)),
    (
        EVENT_MEMORY_CONSOLIDATED,
        (Entity::Memory, Verb::Progressed),
    ),
    (EVENT_MEMORY_FORGOTTEN, (Entity::Memory, Verb::Closed)),
    (EVENT_MEMORY_RECALLED, (Entity::Memory, Verb::Asserted)),
    (EVENT_MEMORY_CALIBRATED, (Entity::Memory, Verb::Asserted)),
    // ── 承诺（commitment）──
    (EVENT_COMMITMENT_OFFERED, (Entity::Commitment, Verb::Opened)),
    (
        EVENT_COMMITMENT_RELEASED,
        (Entity::Commitment, Verb::Closed),
    ),
    (EVENT_COMMITMENT_BROKEN, (Entity::Commitment, Verb::Closed)),
    (
        EVENT_COMMITMENT_ASSERTED,
        (Entity::Commitment, Verb::Asserted),
    ),
    // ── 任务（task）──
    (EVENT_TASK_OPENED, (Entity::Task, Verb::Opened)),
    (EVENT_TASK_PROGRESS, (Entity::Task, Verb::Progressed)),
    (EVENT_TASK_HELD, (Entity::Task, Verb::Held)),
    (EVENT_TASK_ASSERTED, (Entity::Task, Verb::Asserted)),
    (EVENT_TASK_REWORK_CREATED, (Entity::Task, Verb::Asserted)),
    // ── 控制（control）：打断与熔断共用的一格 ──
    (EVENT_CONTROL_OPENED, (Entity::Control, Verb::Opened)),
    // ── 系统与「欲」（system / conation，自主层不加机制）──
    (EVENT_SYSTEM_TRIGGERED, (Entity::System, Verb::Opened)),
    (EVENT_SYSTEM_HEALTH, (Entity::System, Verb::Progressed)),
    (EVENT_CONATION_EXPRESSED, (Entity::Conation, Verb::Opened)),
    // ── 裁决（verdict）──
    (EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted)),
];

/// 这个名字落在哪个格子（`None` = 名字**不在表上**，即网格之外）。
///
/// 只按**精确拼写**查（大小写、前后缀、近似拼写一律 `None`）——判据要能给写侧闸门
/// 一个明确的「不在表上」，而不是「大概是指那一个」。
pub fn event_cell(name: &str) -> Option<Cell> {
    CELLS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, cell)| cell)
}

#[cfg(test)]
#[path = "grid.test.rs"]
mod tests;
