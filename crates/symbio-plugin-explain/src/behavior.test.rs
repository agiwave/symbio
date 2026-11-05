//! `behavior.rs` 的单元测试 —— 成功才记账、决定进幂等键、同决定同理由在执行前被拒。

use super::{ExplainCapture, EXPLAIN_RATIONALE_BEHAVIOR};
use crate::decl::EXPLAIN_RATIONALE;
use serde_json::json;
use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};
use symbio_plugin_api::{EffectEnv, EffectInput, Entity, Event, ToolBehavior, Verb};

fn env() -> EffectEnv<'static> {
    EffectEnv {
        anchor: "t3",
        user_seq: 42,
        principal: "session/a",
        source: "user",
        snapshot: &[],
        applied: &[],
    }
}

fn input(success: bool, args: serde_json::Value) -> EffectInput {
    EffectInput {
        tool: "explain_record".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[EXPLAIN_RATIONALE_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功记录 ⇒ 一条 `Domain × Asserted` 草稿，幂等键带轮锚、调用 id **与决定**。
#[test]
fn a_successful_call_yields_one_rationale_draft() {
    let d = drafts(
        true,
        json!({ "decision": "切换到增量编译", "why": "全量编译超过预算", "evidence": "构建日志" }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, EXPLAIN_RATIONALE);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["decision"], "切换到增量编译");
    assert_eq!(d[0].payload["why"], "全量编译超过预算");
    assert_eq!(d[0].payload["evidence"], "构建日志");
    assert_eq!(d[0].produced_by, Some(42));
    assert_eq!(d[0].key, "x-t3-call-1-切换到增量编译");
}

/// 失败调用没有效果：一条理由不能被「没跑成」的调用记下。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(false, json!({ "decision": "a", "why": "b" })).is_empty());
}

/// 空白字段没有效果（工具侧会先拒，但行为不依赖工具侧兜底）。
#[test]
fn blank_fields_yield_nothing() {
    assert!(drafts(true, json!({ "decision": "  ", "why": "b" })).is_empty());
    assert!(drafts(true, json!({ "decision": "a", "why": "\t" })).is_empty());
}

/// 证据缺省为空串；不是字符串时没有效果（工具侧会先拒）。
#[test]
fn evidence_defaults_and_rejects_non_strings() {
    let d = drafts(true, json!({ "decision": "a", "why": "b" }));
    assert_eq!(d[0].payload["evidence"], "");
    assert!(drafts(true, json!({ "decision": "a", "why": "b", "evidence": 1 })).is_empty());
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(true, json!({ "decision": "a", "why": "b" }));
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

fn judgement(args: &serde_json::Value, facts: &[Event]) -> BeforeDecision {
    ExplainCapture.before(&BeforeRequest {
        tool: "explain_record",
        call_id: "call-2",
        args,
        principal: "session/a",
        declared: &[EXPLAIN_RATIONALE_BEHAVIOR],
        facts,
        readings: BeforeReadings::default(),
    })
}

fn recorded(decision: &str, why: &str) -> Event {
    Event::pending(
        "x-t3-call-0",
        EXPLAIN_RATIONALE,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({ "decision": decision, "why": why, "evidence": "" }))
}

/// 会话里没有这条记录 ⇒ 放行。
#[test]
fn a_fresh_record_is_allowed() {
    assert!(matches!(
        judgement(
            &json!({ "decision": "a", "why": "b" }),
            &[recorded("x", "y")]
        ),
        BeforeDecision::Allow
    ));
}

/// 同一决定 + 同一理由 ⇒ 执行前拒绝，拒因点名这条理由。
#[test]
fn a_repeated_record_is_refused_before_execution() {
    let BeforeDecision::Refuse(refusal) = judgement(
        &json!({ "decision": " a ", "why": "b" }),
        &[recorded("a", "b")],
    ) else {
        panic!("同一决定 + 同一理由应被拒绝");
    };
    let text = refusal.reason;
    assert!(text.contains("已经记录过"), "拒因应说清是重复：{text}");
    assert!(text.contains("a — b"), "拒因应点名这一对：{text}");
}

/// 同一决定换一条理由**不是**重复：那是同一决定的新理由，放行。
#[test]
fn the_same_decision_with_a_new_why_is_not_a_duplicate() {
    assert!(matches!(
        judgement(
            &json!({ "decision": "a", "why": "c" }),
            &[recorded("a", "b")]
        ),
        BeforeDecision::Allow
    ));
}
