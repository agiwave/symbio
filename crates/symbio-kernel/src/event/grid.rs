//! 事件名字表 —— **名字 → 格位（实体 × 动词）** 收成一份数据，判它的位置在唯一写入口。
//!
//! ## 它补的是哪一格
//!
//! 网格的**形状**由两个闭集枚举钉死（[`Entity`] 11 行 × [`Verb`] 5 列，F5），
//! 名字却是**字符串常量**（[plan/01 §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：
//! 「名字是数据，加名字不加枚举」）。于是「`user.message` 落在 `turn × opened`」这条
//! **对应关系**此前只有两个载体：常量头上的散文说明，与各构造点手填的 `entity` / `verb`。
//! 散文不判、构造点没人核 ⇒ 漂了没有任何信号。而投影是**按格子**取事实的
//! （`event_node_of` 的 `match` 三目键就是 `(entity, verb, kind)`），
//! 漂一寸的表现是**静默少一条事实**——不是报错。
//!
//! 本模块把这条关系落成 `CELLS` 一张表，判据是 [`event_cell`]，强制点在
//! **唯一写入口**内（会话事实源的写侧闸门，报错形态见
//! `AppendError::OffGrid`）。
//!
//! ## 为什么是「表 + 入库闸」，不是 `EventKind` 枚举
//!
//! 审计稿（
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
//! `EventEnvelope::fallback_timestamp` 的教训记录）。
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
    Entity, EventDecl, FactRole, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS,
    EVENT_ASSISTANT_REPORTED, EVENT_AUTHZ_DENIED, EVENT_BRANCH_SELECTED, EVENT_COMMITMENT_ASSERTED,
    EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OFFERED, EVENT_COMMITMENT_RELEASED, EVENT_FAILED,
    EVENT_MESSAGE_ADDED, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_REQUEST,
    EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT, EVENT_SESSION_CONSOLIDATED, EVENT_TURN_BRANCH,
    EVENT_TURN_DISPATCH, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE, EVENT_VERDICT_ASSERTED,
};

/// 网格上的一个格子：实体（行）× 动词（列）。
pub type Cell = (Entity, Verb);

/// 名字 → 格位（与角色）。加名字 = 加一行（数据变化，F5）；段序与
/// [`super`] 的名字表一致，一族一段。
///
/// `roles` 那一维是 core 守卫判据的输入面（DE-024 D-203）：判据问「这条事实扮演
/// 什么角色」，不再自己抄名字。**迁移期的两份表**——owner 插件的 `EventDecl` 与这张
/// 内核表必须给同一个角色；内核表归零后只剩 owner 那一份，`seal` 的
/// `Gap::UnmannedRole` 保证每一档角色都有人住。
const CELLS: &[EventDecl] = &[
    // ── 对话面（turn）──
    EventDecl::with_roles(
        EVENT_USER_MESSAGE,
        (Entity::Turn, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
    // 派发档是「这一轮以什么方式运行」的断言，与阶段事实同格。
    EventDecl::with_roles(
        EVENT_TURN_DISPATCH,
        (Entity::Turn, Verb::Asserted),
        &[FactRole::DeclaresBudget],
    ),
    EventDecl::new(EVENT_TURN_SUPPLEMENTED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_ASSISTANT_REPORTED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_ASSISTANT_OPENING, (Entity::Turn, Verb::Asserted)),
    EventDecl::with_roles(
        EVENT_ASSISTANT_FINAL,
        (Entity::Turn, Verb::Closed),
        &[FactRole::FinalizesTurn],
    ),
    EventDecl::new(EVENT_ASSISTANT_FALLBACK, (Entity::Turn, Verb::Closed)),
    EventDecl::new(EVENT_ASSISTANT_PROGRESS, (Entity::Turn, Verb::Progressed)),
    // 分支族（重试留下的几支与「此刻读哪一支」）：两支都**不删任何东西**，所以它们
    // 不是 `closed`（那一格是「这一轮的产出定稿」）；它们是「关于这一轮的断言」——
    // 与上面配置 / 派发同格。落 `asserted` 的另一半理由是 I2：断言类必须带
    // `produced_by`，而「这一支存在」与「现在读这一支」正是靠溯源才能被验真。
    EventDecl::new(EVENT_TURN_BRANCH, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_BRANCH_SELECTED, (Entity::Turn, Verb::Asserted)),
    // ── 协议族（对外调用，）─────────────────────────────────────────
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
    // （`chat.assistant.progress`），协议事实是机器对机器的账。I2 现在按格子判
    // `progressed`（DE-024 D-203）——落那一格就**必然**要溯源，不再有「静默失去
    // 溯源要求」这回事；`asserted` 同样整格要求溯源，五条都自动受约束。
    EventDecl::new(EVENT_REQUEST, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_SENT, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RESPONSE, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_FAILED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RETRY, (Entity::Turn, Verb::Asserted)),
    // 否定事实与收束格**不同格**（`asserted` vs `closed`）：被拒的一轮仍然未收束。
    // 它不是收束，但回答「这一轮为什么停」——角色停在这一档上（`StopsTurn`）。
    EventDecl::with_roles(
        EVENT_AUTHZ_DENIED,
        (Entity::Turn, Verb::Asserted),
        &[FactRole::StopsTurn],
    ),
    // ── 产物（artifact）──
    EventDecl::new(EVENT_ARTIFACT_ADDED, (Entity::Artifact, Verb::Asserted)),
    // ── 消息列表的变更（message，D-053 / D-054 的闭集四格）──
    EventDecl::new(EVENT_MESSAGE_ADDED, (Entity::Message, Verb::Opened)),
    EventDecl::new(EVENT_MESSAGE_EDITED, (Entity::Message, Verb::Asserted)),
    EventDecl::new(EVENT_MESSAGE_REMOVED, (Entity::Message, Verb::Closed)),
    EventDecl::new(
        EVENT_SESSION_CONSOLIDATED,
        (Entity::Message, Verb::Progressed),
    ),
    // ── 记忆（memory）──
    // `memory.encoded` 与 `memory.calibrated` 的格位都随 owner 的声明走
    // （`plugins/loop::control_decl` + 清单 `shared_events`，DE-024 D-204 / D-205）：
    // 内核这一节现在是空的。
    // ── 承诺（commitment）──
    EventDecl::new(EVENT_COMMITMENT_OFFERED, (Entity::Commitment, Verb::Opened)),
    EventDecl::new(
        EVENT_COMMITMENT_RELEASED,
        (Entity::Commitment, Verb::Closed),
    ),
    EventDecl::new(EVENT_COMMITMENT_BROKEN, (Entity::Commitment, Verb::Closed)),
    EventDecl::new(
        EVENT_COMMITMENT_ASSERTED,
        (Entity::Commitment, Verb::Asserted),
    ),
    // ── 任务（task）──
    // 内核这一节现在是空的：五格同属 `plugins/loop`（写方与调度者都在那儿），
    // `session` 作为第二个消费方经清单生成物读它们（DE-024 D-204 / D-206，判据 CS-007）。
    // ── 控制（control）──
    // 打断与熔断共用的那一格（`task.controlled`）随 owner 的声明走：它的写方在两个
    // 插件各一个，名字因此登记进 owner 清单（DE-024 D-204），不在内核表上。
    // ── 裁决（verdict）──
    EventDecl::new(EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted)),
];

/// 这张表本身——迁移期写闸与判据的**回退词汇面**。
///
/// owner 的声明表优先（`Frozen::holds_role` / `event_cell` 先查它）；内核表随
/// 业务名字逐条迁出而缩小，归零后本函数与那条回退臂一起删除（DE-024 D-203 裁决三）。
pub fn kernel_vocab() -> &'static [EventDecl] {
    CELLS
}

/// 这个名字落在哪个格子（`None` = 名字**不在表上**，即网格之外）。
///
/// 只按**精确拼写**查（大小写、前后缀、近似拼写一律 `None`）——判据要能给写侧闸门
/// 一个明确的「不在表上」，而不是「大概是指那一个」。
pub fn event_cell(name: &str) -> Option<Cell> {
    CELLS.iter().find(|e| e.name == name).map(|e| e.cell)
}

#[cfg(test)]
#[path = "grid.test.rs"]
mod tests;
