//! 事实类型网格 —— 实体 × 动词，**取值穷举**。
//!
//! ## 为什么是网格而不是一张名字列表
//!
//! 列表会膨胀：每加一个能力就往里塞一个字符串，无法判断"该不该加"。
//! 网格是**封闭**的：实体有限、动词有限，新增事实 = 在网格里占一格。
//! 这一条是 v2「用封闭坐标系吸收开放能力清单」在本仓的落地
//! （见 `docs/plan/02-能力坐标系.md`）。
//!
//! ## 10 实体 × 5 动词
//!
//! | 实体 \ 动词 | `opened` | `progressed` | `held` | `closed` | `asserted` |
//! |---|---|---|---|---|---|
//! | `turn` | `turn.user_message` | `turn.state_changed` | — | `turn.assistant_final`<br/>`turn.assistant_fallback` | — |
//! | `task` | `task.created` | `task.progressed` | `task.held` | `task.completed`<br/>`task.failed` | `task.verified` |
//! | `artifact` | — | — | — | — | `artifact.added` |
//! | `memory` | `memory.encoded` | `memory.consolidated` | — | `memory.forgotten` | `memory.recalled` |
//! | `commitment` | `commitment.offered` | — | — | `commitment.released` | `commitment.asserted` |
//! | `conation` | `conation.expressed` | `conation.evaluated` | — | `conation.dropped` | `conation.gated` |
//! | `system` | `system.triggered` | `system.health` | — | — | — |
//!
//! 当前已点亮的格子是本枚举的取值；`—` 的格子尚未点亮，**但在网格内**——
//! 点亮它们只需要加枚举取值，**不需要改任何机制**（断言 A1 检查的正是这一点）。
//!
//! ## 与 v2 完整网格的差距（诚实标注）
//!
//! v2 的完整网格有 10 个实体，本仓当前**先点亮 7 个**（上表）。
//! 未列入的 `verdict` / `thread` / `control` 三个实体在现行体系里尚无对应事实，
//! 待其能力（评审 / 断点 / 控制）真正落地时**追加枚举取值**——这仍属"加格子"，
//! 不改机制。**不要为尚未存在的能力预先占位**：空枚举值会让人以为系统已经支持它。

use serde::{Deserialize, Serialize};

/// 事实类型 —— 全系统可观测事实的**穷举**目录。
///
/// 每个取值形如 `<实体>.<动词>`，与 v2 的事件网格一一对应。
/// [`FactKind::ALL`] 是可被断言 A1 核对的全集（机制表的那一行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    // ---- turn（轮次：对话基线）----
    /// 用户消息成为事实（对应 `open`）
    TurnUserMessage,
    /// 轮次状态变更（对应 `progress`）
    TurnStateChanged,
    /// 本轮唯一的最终发言（对应 `close`）
    TurnAssistantFinal,
    /// 兜底发言（超预算 / 失败时的到点必答，对应 `close`）
    TurnAssistantFallback,

    // ---- task（任务：多步与拆解）----
    /// 任务被创建（对应 `open`）
    TaskCreated,
    /// 任务推进（对应 `progress`）
    TaskProgressed,
    /// 任务被挂起（对应 `hold`）
    TaskHeld,
    /// 任务完成（对应 `close`）
    TaskCompleted,
    /// 任务失败（对应 `close`）
    TaskFailed,
    /// 任务被验证（对应 `assert`）—— 断言类，须带溯源
    TaskVerified,

    // ---- artifact（产物）----
    /// 产物被登记（对应 `assert`）—— 断言类，须带溯源
    ArtifactAdded,

    // ---- memory（长期记忆，S06）----
    /// 记忆被编码（对应 `open`）
    MemoryEncoded,
    /// 记忆被巩固（对应 `progress`）
    MemoryConsolidated,
    /// 记忆被遗忘（对应 `close`；非物理删除，只是投影不再包含）
    MemoryForgotten,
    /// 记忆被召回（对应 `assert`）
    MemoryRecalled,

    // ---- commitment（对等承诺，S08）----
    /// 承诺被提出（对应 `open`）
    CommitmentOffered,
    /// 承诺被撤销（对应 `close`）
    CommitmentReleased,
    /// 承诺被声明（对应 `assert`）—— 断言类，须带溯源
    CommitmentAsserted,

    // ---- conation（「欲」，S12）----
    /// 欲被表达（对应 `open`）—— 「想要」成为可审计的事实
    ConationExpressed,
    /// 欲被评估（对应 `progress`）
    ConationEvaluated,
    /// 欲被丢弃（对应 `close`；平凡路径：关闭「欲」时走这里）
    ConationDropped,
    /// 欲通过闸门（对应 `assert`）—— 断言类，须带溯源
    ConationGated,

    // ---- system（系统信号：定时与健康）----
    /// 系统触发（对应 `open`）
    SystemTriggered,
    /// 系统健康（对应 `progress`）
    SystemHealth,
}

impl FactKind {
    /// **全集** —— 断言 A1（机制表守恒）的核对依据。
    ///
    /// 新增取值必须同时加进这里，否则 A1 会报"代码里的键不在机制表内"。
    /// 这不是形式主义：它把"扩展要加数据、不要加机制"从口头纪律变成可执行的失败信号。
    pub const ALL: &'static [FactKind] = &[
        FactKind::TurnUserMessage,
        FactKind::TurnStateChanged,
        FactKind::TurnAssistantFinal,
        FactKind::TurnAssistantFallback,
        FactKind::TaskCreated,
        FactKind::TaskProgressed,
        FactKind::TaskHeld,
        FactKind::TaskCompleted,
        FactKind::TaskFailed,
        FactKind::TaskVerified,
        FactKind::ArtifactAdded,
        FactKind::MemoryEncoded,
        FactKind::MemoryConsolidated,
        FactKind::MemoryForgotten,
        FactKind::MemoryRecalled,
        FactKind::CommitmentOffered,
        FactKind::CommitmentReleased,
        FactKind::CommitmentAsserted,
        FactKind::ConationExpressed,
        FactKind::ConationEvaluated,
        FactKind::ConationDropped,
        FactKind::ConationGated,
        FactKind::SystemTriggered,
        FactKind::SystemHealth,
    ];

    /// 线上词形（`<实体>.<动词>`）—— 与 v2 网格逐字一致。
    pub fn wire(&self) -> &'static str {
        match self {
            FactKind::TurnUserMessage => "turn.user_message",
            FactKind::TurnStateChanged => "turn.state_changed",
            FactKind::TurnAssistantFinal => "turn.assistant_final",
            FactKind::TurnAssistantFallback => "turn.assistant_fallback",
            FactKind::TaskCreated => "task.created",
            FactKind::TaskProgressed => "task.progressed",
            FactKind::TaskHeld => "task.held",
            FactKind::TaskCompleted => "task.completed",
            FactKind::TaskFailed => "task.failed",
            FactKind::TaskVerified => "task.verified",
            FactKind::ArtifactAdded => "artifact.added",
            FactKind::MemoryEncoded => "memory.encoded",
            FactKind::MemoryConsolidated => "memory.consolidated",
            FactKind::MemoryForgotten => "memory.forgotten",
            FactKind::MemoryRecalled => "memory.recalled",
            FactKind::CommitmentOffered => "commitment.offered",
            FactKind::CommitmentReleased => "commitment.released",
            FactKind::CommitmentAsserted => "commitment.asserted",
            FactKind::ConationExpressed => "conation.expressed",
            FactKind::ConationEvaluated => "conation.evaluated",
            FactKind::ConationDropped => "conation.dropped",
            FactKind::ConationGated => "conation.gated",
            FactKind::SystemTriggered => "system.triggered",
            FactKind::SystemHealth => "system.health",
        }
    }

    /// 实体名（`wire()` 里 `.` 之前那段）。
    ///
    /// 消费方对未知实体必须安全（`switch` 有兜底）——这是"新增事件准入三问"之一。
    pub fn entity(&self) -> &'static str {
        match self.wire().split_once('.') {
            Some((entity, _)) => entity_head(entity),
            None => "unknown",
        }
    }

    /// 是否为**断言类**事实 —— 这类事实的 `caused_by` 必须非空（不变量 I2）。
    ///
    /// 断言类 = "某主体声称了某件关于已发生事实的事"，它**必须能追溯到产生它的输入**，
    /// 否则就是凭空声明。这条判据由断言 A2 强制。
    pub fn is_assertion(&self) -> bool {
        matches!(
            self,
            FactKind::TaskVerified
                | FactKind::ArtifactAdded
                | FactKind::MemoryRecalled
                | FactKind::CommitmentAsserted
                | FactKind::ConationGated
        )
    }
}

/// 表头实体的静态串（避免 `wire()` 的 `&'static str` 借用了临时切片）。
fn entity_head(entity: &str) -> &'static str {
    match entity {
        "turn" => "turn",
        "task" => "task",
        "artifact" => "artifact",
        "memory" => "memory",
        "commitment" => "commitment",
        "conation" => "conation",
        "system" => "system",
        _ => "unknown",
    }
}
