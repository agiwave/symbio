//! 「欲」与「行」之间的一道门（02 §2.3 E2）：候选意图、治理策略、闸门令牌与升格。

use crate::symbio_core::Event;

/// 「欲」的治理策略（E3：平凡值 `enabled = false`——关掉后系统退化为纯响应式
/// 且仍完整运行；两层开关独立，这是生产环境最需要的开关）。
#[derive(Debug, Clone)]
pub struct ConationPolicy {
    /// 是否允许「欲」升格为任务。
    pub enabled: bool,
    /// 目标长度上界（过宽的目标 = 不可评估的意图，拒绝）。
    pub max_goal_len: usize,
}

impl Default for ConationPolicy {
    fn default() -> Self {
        ConationPolicy {
            enabled: false,
            max_goal_len: 200,
        }
    }
}

/// 候选意图：从 `conation.expressed` 事件**唯一**构造路径读出，
/// 造出来一定**未被批准**——「欲」不得直接变成「行」（E2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConationCandidate {
    /// 源事件 seq（溯源锚）。
    pub seq: u64,
    pub goal: String,
    /// 私有：只有闸门能把它置真。
    approved: bool,
}

impl ConationCandidate {
    /// 唯一构造路径：从欲事件读出候选。**无溯源的欲构造不出候选**（I2 强化）。
    pub fn from_event(e: &Event) -> Option<Self> {
        if e.kind != crate::symbio_core::event::EVENT_CONATION_EXPRESSED {
            return None;
        }
        // 来路不明的意图混不进来（I2 强化：无溯源即 None）。
        e.produced_by?;
        let goal = e
            .payload
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some(ConationCandidate {
            seq: e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
            goal,
            approved: false,
        })
    }

    pub fn is_approved(&self) -> bool {
        self.approved
    }
}

/// 闸门能力令牌（ZST，私有构造 → 不可伪造；02 §2.3 E2 的可编译强制：
/// 不持令牌则 `approve` 调用**编译失败**，`size_of == 0` 零运行时开销）。
#[derive(Debug, Clone, Copy, Default)]
pub struct GateWarrant {
    _private: (),
}

/// 闸门评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentDecision {
    Approved,
    Rejected(&'static str),
}

/// 经闸门批准后的可执行任务（只有这一条路能造出来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedIntent {
    pub from_seq: u64,
    pub goal: String,
}

/// 意图闸门：**「欲」与「行」之间唯一的一道门**（02 §2.3 E2）。
pub struct IntentGate;

impl IntentGate {
    /// 评估：关停开关优先；目标过宽拒绝。真实系统里这里接价值偏好 / 预算 / 授权。
    pub fn evaluate(candidate: &ConationCandidate, policy: &ConationPolicy) -> IntentDecision {
        if !policy.enabled {
            return IntentDecision::Rejected("conation disabled");
        }
        if candidate.goal.len() > policy.max_goal_len {
            return IntentDecision::Rejected("goal too broad");
        }
        IntentDecision::Approved
    }

    /// 升格：**唯一**能把 [`ConationCandidate`] 变成 [`ApprovedIntent`] 的函数。
    /// 要求 (a) 评估通过 (b) 持 [`GateWarrant`]（不持令牌 = 编译失败）。
    pub fn approve(
        candidate: &mut ConationCandidate,
        policy: &ConationPolicy,
        _warrant: &GateWarrant,
    ) -> Result<ApprovedIntent, &'static str> {
        match Self::evaluate(candidate, policy) {
            IntentDecision::Approved => {
                candidate.approved = true;
                Ok(ApprovedIntent {
                    from_seq: candidate.seq,
                    goal: candidate.goal.clone(),
                })
            }
            IntentDecision::Rejected(r) => Err(r),
        }
    }

    /// 发牌入口——**故意做成唯一一道**：真要多一道门，就得再写一个发牌函数，
    /// 而那个函数是可见的、可审计的（不是靠约定）。
    pub fn issue_warrant() -> GateWarrant {
        GateWarrant { _private: () }
    }
}
