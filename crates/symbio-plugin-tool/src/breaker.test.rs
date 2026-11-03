//! 熔断执行体的行为判据（验收 1 / 2 / 4）——与 `breaker.rs` 同目录外置，
//! 遵循仓内「生产文件不内联测试」的布局约定。

use super::*;

/// 验收 1：未授权 ⇒ `Refuse`；超时**先于**授权判定——判据顺序本身是规格。
#[test]
fn refuses_unauthorized() {
    assert_eq!(
        CircuitBreaker.gate(false, 0, 500, 1_000, 10, 80),
        GateDecision::Refuse
    );
    assert_eq!(
        CircuitBreaker.gate(false, 0, 500, 1_000, 200, 80),
        GateDecision::Break {
            reason: "gate-timeout"
        }
    );
}

/// 验收 2 + 4：预算耗尽 / 超时熔断；预算放宽 ⇒ 放行。
#[test]
fn break_table_and_fact_shape() {
    let gate = CircuitBreaker;
    assert_eq!(
        gate.gate(true, 900, 500, 1_000, 10, 80),
        GateDecision::Break {
            reason: "budget-exhausted"
        }
    );
    assert_eq!(
        gate.gate(true, 900, 500, 10_000, 10, 80),
        GateDecision::Allow
    );
    assert_eq!(
        gate.gate(true, 0, 500, 10_000, 200, 80),
        GateDecision::Break {
            reason: "gate-timeout"
        }
    );

    let fact = gate.break_fact("tc-1", "budget-exhausted");
    assert_eq!(fact.key, "cb-tc-1");
    assert_eq!(fact.kind, EVENT_TASK_CONTROLLED);
    assert_eq!(fact.entity, Entity::Control);
    assert_eq!(fact.verb, Verb::Opened);
    assert_eq!(fact.payload["reason"], "budget-exhausted");
}
