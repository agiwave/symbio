//! 会话运行态的**事实投影**：上一轮结局从生命周期事实折出。
//!
//! [D-083 §5.2](../../../../docs/003-design/001-核心架构.md "D-083") 的落地侧：结局在网格里
//! 有确切的证据格，因此它是投影而不是又一份状态；`working` / `warning` 是点名论证过的
//! 进程局部量，不在这里。
//!
//! ## 哪些格是生命周期证据
//!
//! | 格 | 结局 |
//! |---|---|
//! | `user.message`（开轮） | 清空：新一轮开始，上一轮结局作废 |
//! | `chat.assistant.final` | 载荷 `status`：`aborted` / `failed` / 其余按 `completed` |
//! | `chat.assistant.fallback` | `failed`，错误取载荷 `why` |
//! | `chat.assistant.progress` | `completed`（等待用户的那一轮以 `Ok` 收尾） |
//! | 根 run 节点的 `message.edited` / `message.added` | `Failed` / `Aborted` 即失败/中止收口 |
//!
//! 按**完整史**扫描、取最后一条证据：压缩只取代消息列表，不取代轮次生命周期
//! （[D-085](README.md "D-085") 的同一条取向）。非生命周期事件（产出、汇报、
//! 工具、任务、记忆、消息编辑里不承载根 run 节点的那些）一律不改变结局。

use crate::symbio_core::chat_node as cm;
use crate::symbio_core::{
    Event, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_ASSISTANT_PROGRESS,
    EVENT_MESSAGE_ADDED, EVENT_MESSAGE_EDITED, EVENT_USER_MESSAGE,
};

/// 上一轮结局——与 `attributes.outcome` 的词表同构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeOutcome {
    Completed,
    Aborted,
    /// 与线上形状一致：错误文案只在失败时有意义。
    Failed {
        error: Option<String>,
    },
}

impl RuntimeOutcome {
    /// 投影 → 节点属性的词表映射（`outcome`, `error`）。
    fn as_wire(&self) -> (&'static str, Option<String>) {
        use crate::plugins::session::plugin::{OUTCOME_ABORTED, OUTCOME_COMPLETED, OUTCOME_FAILED};
        match self {
            RuntimeOutcome::Completed => (OUTCOME_COMPLETED, None),
            RuntimeOutcome::Aborted => (OUTCOME_ABORTED, None),
            RuntimeOutcome::Failed { error } => (OUTCOME_FAILED, error.clone()),
        }
    }

    /// 运行态视图（`SessionRuntime` 的 `outcome` / `error` 两格）。
    pub(crate) fn into_state(self) -> (String, Option<String>) {
        let (outcome, error) = self.as_wire();
        (outcome.to_string(), error)
    }
}

/// 事实侧运行态：目前只有结局（`working` / `warning` 不是事实，见模块文档）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RuntimeFacts {
    pub(super) outcome: Option<RuntimeOutcome>,
}

/// 从**完整史**折出运行态事实。
///
/// 纯函数：形参只有事件切片，可穷举测试；同一序列恒得同一结果。
pub(crate) fn project(events: &[Event]) -> RuntimeFacts {
    let mut facts = RuntimeFacts::default();
    for e in events {
        if let Some(step) = outcome_step(e) {
            facts.outcome = step;
        }
    }
    facts
}

/// 一条事件对结局的作用：`None` = 不是证据；`Some(None)` = 开新轮清空；
/// `Some(Some(_))` = 一轮收束。
///
/// 结局问的是**结果面**，所以读 `status`（三值）；「为什么停」读 `reason`
/// （闭集，）——两个问题两个字段，本函数只问前者。
fn outcome_step(e: &Event) -> Option<Option<RuntimeOutcome>> {
    match e.kind.as_str() {
        EVENT_USER_MESSAGE => Some(None),
        EVENT_ASSISTANT_FINAL => Some(Some(match payload_str(e, "status").as_deref() {
            Some("aborted") => RuntimeOutcome::Aborted,
            // 收束格的 `failed` 先于兜底格存在时也要认——两格读同一个状态词。
            Some("failed") => RuntimeOutcome::Failed {
                error: payload_str(e, "detail"),
            },
            _ => RuntimeOutcome::Completed,
        })),
        EVENT_ASSISTANT_FALLBACK => Some(Some(RuntimeOutcome::Failed {
            error: payload_str(e, "detail"),
        })),
        // 等待用户：本轮以 Ok 收尾（恢复续写是下一轮的事），与现网结局一致。
        EVENT_ASSISTANT_PROGRESS => Some(Some(RuntimeOutcome::Completed)),
        EVENT_MESSAGE_EDITED => e
            .payload
            .get("message")
            .and_then(root_run_outcome)
            .map(Some),
        EVENT_MESSAGE_ADDED => {
            let nodes = e.payload.get("messages")?.as_array()?;
            // 一次诞生一批：只认根 run 节点，多条时以最后一条为准。
            let mut step = None;
            for node in nodes {
                if let Some(outcome) = root_run_outcome(node) {
                    step = Some(Some(outcome));
                }
            }
            step
        }
        _ => None,
    }
}

/// 节点是不是「失败 / 中止收口的根 run 节点」；是则给出对应结局。
///
/// 限定 `is_run()` + 无父节点：`persist_failure` / `converge_inflight` 只定稿这个根，
/// 子节点的 Completed 与正文节点的状态都不是轮次结局的证据。
fn root_run_outcome(node: &serde_json::Value) -> Option<RuntimeOutcome> {
    let node: cm::ChatNode = serde_json::from_value(node.clone()).ok()?;
    if !(node.is_run() && node.parent_id.is_none()) {
        return None;
    }
    match node.status {
        Some(cm::NodeStatus::Failed) => Some(RuntimeOutcome::Failed {
            error: node.error.clone(),
        }),
        Some(cm::NodeStatus::Aborted) => Some(RuntimeOutcome::Aborted),
        _ => None,
    }
}

/// 载荷里的非空字符串（空串视为「没记」，与事件层 `payload_str` 同口径）。
fn payload_str(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "runtime.test.rs"]
mod tests;
