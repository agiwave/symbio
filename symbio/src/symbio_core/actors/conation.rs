//! 「欲」与「行」之间的一道门（02 §2.3 E2）：候选意图、治理策略、闸门令牌与升格。

use crate::symbio_core::authz::EffectiveMatrix;
use crate::symbio_core::governance::Capability;
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

/// 闸门评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentDecision {
    Approved,
    Rejected(&'static str),
}

/// 经闸门批准后的可执行任务（**只有** [`IntentGate::approve` 能造出来：字段私有，
/// 模块外连结构体字面量都写不出——这是 E2「欲不得直接变成行」的编译期兑现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedIntent {
    from_seq: u64,
    goal: String,
}

impl ApprovedIntent {
    /// 溯源锚：升格自哪一条 `conation.expressed`。
    pub fn source_seq(&self) -> u64 {
        self.from_seq
    }

    /// 被批准的目标文本。
    pub fn goal(&self) -> &str {
        &self.goal
    }
}

/// 意图闸门：**「欲」与「行」之间唯一的一道门**（02 §2.3 E2）。
pub struct IntentGate;

impl IntentGate {
    /// 评估：关停开关优先，其次授权（升格就是「定义工作」），最后目标宽度。
    ///
    /// 授权**不是**一句声明：判据取 [`EffectiveMatrix::can_write`] 那一份
    /// （`define.work`），与本仓任务表写侧闸同一把尺。预算与时延不在这里判——
    /// 它们是**执行面**的闸（`CircuitBreaker` 在真动作上判），把意图升格成任务
    /// 本身不花预算。
    pub(crate) fn evaluate(
        candidate: &ConationCandidate,
        policy: &ConationPolicy,
        grants: &EffectiveMatrix,
        principal: &str,
    ) -> IntentDecision {
        if !policy.enabled {
            return IntentDecision::Rejected("conation disabled");
        }
        if !grants.can_write(principal, Capability::DefineWork) {
            return IntentDecision::Rejected("unauthorized: define.work");
        }
        if candidate.goal.len() > policy.max_goal_len {
            return IntentDecision::Rejected("goal too broad");
        }
        IntentDecision::Approved
    }

    /// 升格：**唯一**能把 [`ConationCandidate`] 变成 [`ApprovedIntent`] 的函数
    /// （`ApprovedIntent` 字段私有，绕过它造不出可执行任务）。
    pub(crate) fn approve(
        candidate: &mut ConationCandidate,
        policy: &ConationPolicy,
        grants: &EffectiveMatrix,
        principal: &str,
    ) -> Result<ApprovedIntent, &'static str> {
        match Self::evaluate(candidate, policy, grants, principal) {
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
}
