//! 时延 / 预算熔断：执行体内部的开窗判据。
//!
//! 环**不判快慢**，也不据快慢拒发：它只把中性输入（授权结论 / 已耗 / 本次申请 /
//! 总预算）随债交过来。判据、「熔断中不发」以及那条 `control × opened` 事实都由
//! 本执行体自己落（[DE-039 §6](../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断")）。

use crate::declarations::EVENT_TASK_CONTROLLED;
use serde_json::json;
use symbio_plugin_api::{Entity, FactDraft, Verb};

/// 一次开窗判定的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateDecision {
    /// 未授权——这一笔**不产事件**（验收 1：拒绝且无事件）。
    Refuse,
    /// 熔断（预算耗尽 / 判定超时）——必须落 `control × opened`，不允许静默继续
    /// （验收 2）。
    Break { reason: &'static str },
    /// 放行执行。
    Allow,
}

pub(crate) struct CircuitBreaker;

impl CircuitBreaker {
    /// 外部执行闸门。
    ///
    /// - `authorized`：主体是否持外部执行能力（环读矩阵后交给本执行体的中性值）；
    /// - `spent_ms` / `requested_ms` / `budget_ms`：已耗 / 本次申请 / 总预算；
    /// - `elapsed_ms`：**本执行体**从受理到判定的时长；超过 `elapsed_budget_ms` 即熔断
    ///   （有事件的超时，不是静默失效；I3）；
    /// - `elapsed_budget_ms`：这份输入的时延预算（输入是中性值，取反射档）。
    pub(crate) fn gate(
        &self,
        authorized: bool,
        spent_ms: u64,
        requested_ms: u64,
        budget_ms: u64,
        elapsed_ms: u64,
        elapsed_budget_ms: u64,
    ) -> GateDecision {
        if elapsed_ms > elapsed_budget_ms {
            return GateDecision::Break {
                reason: "gate-timeout",
            };
        }
        if !authorized {
            return GateDecision::Refuse;
        }
        if spent_ms.saturating_add(requested_ms) > budget_ms {
            return GateDecision::Break {
                reason: "budget-exhausted",
            };
        }
        GateDecision::Allow
    }

    /// 熔断事实（复用 `control/opened` 格，载荷 `reason` 区分打断与熔断）。
    ///
    /// 幂等键按**这一笔债**（`idempotency_key`）给：同一笔重放不会落第二条。
    pub(crate) fn break_fact(&self, idempotency_key: &str, reason: &str) -> FactDraft {
        FactDraft {
            key: format!("cb-{idempotency_key}"),
            kind: EVENT_TASK_CONTROLLED.to_string(),
            entity: Entity::Control,
            verb: Verb::Opened,
            payload: json!({ "reason": reason }),
            produced_by: None,
        }
    }
}

#[cfg(test)]
#[path = "breaker.test.rs"]
mod tests;
