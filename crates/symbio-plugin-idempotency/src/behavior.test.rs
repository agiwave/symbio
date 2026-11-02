//! `behavior.rs` 的单元测试 —— 成功才记账、同键在执行前被拒、同轮同键撞同一个事件 id。

use super::{IdempotencyRecord, IDEMPOTENCY_RECORD_BEHAVIOR};
use crate::decl::IDEMPOTENCY_RECORDED;
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
        tool: "idempotent_record".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[IDEMPOTENCY_RECORD_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功记录 ⇒ 一条 `Domain × Asserted` 草稿，幂等键是 `i-{anchor}-{key}`。
#[test]
fn a_successful_call_yields_one_record_draft() {
    let d = drafts(true, json!({ "key": "build-42", "value": "compiled" }));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, IDEMPOTENCY_RECORDED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["key"], "build-42");
    assert_eq!(d[0].payload["value"], "compiled");
    assert_eq!(d[0].produced_by, Some(42));
    assert_eq!(d[0].key, "i-t3-build-42");
}

/// 键里不带调用 id：同一轮同键的两次调用必须撞成同一个事件 id。
#[test]
fn the_key_does_not_depend_on_the_call_id() {
    let a = {
        let mut i = input(true, json!({ "key": "k", "value": "v" }));
        i.call_id = "call-a".into();
        symbio_plugin_api::apply_effects(&i.call(&env()))
    };
    let b = {
        let mut i = input(true, json!({ "key": "k", "value": "v" }));
        i.call_id = "call-b".into();
        symbio_plugin_api::apply_effects(&i.call(&env()))
    };
    assert_eq!(a[0].key, b[0].key);
}

/// 失败调用没有效果：一条键不能被「没跑成」的调用占掉。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(false, json!({ "key": "k", "value": "v" })).is_empty());
}

/// 空白字段没有效果（工具侧会先拒，但行为不依赖工具侧兜底）。
#[test]
fn blank_fields_yield_nothing() {
    assert!(drafts(true, json!({ "key": "  ", "value": "v" })).is_empty());
    assert!(drafts(true, json!({ "key": "k", "value": "\t" })).is_empty());
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(true, json!({ "key": "k", "value": "v" }));
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

fn judgement(args: &serde_json::Value, facts: &[Event]) -> BeforeDecision {
    IdempotencyRecord.before(&BeforeRequest {
        tool: "idempotent_record",
        call_id: "call-2",
        args,
        principal: "session/a",
        declared: &[IDEMPOTENCY_RECORD_BEHAVIOR],
        facts,
        readings: BeforeReadings::default(),
    })
}

fn recorded(key: &str) -> Event {
    Event::pending(
        "i-t3-k",
        IDEMPOTENCY_RECORDED,
        Entity::Domain,
        Verb::Asserted,
        3,
        "session/a",
    )
    .with_payload(json!({ "key": key, "value": "v" }))
}

/// 会话里没有这个键 ⇒ 放行。
#[test]
fn a_fresh_key_is_allowed() {
    assert!(matches!(
        judgement(&json!({ "key": "k2", "value": "v" }), &[recorded("k")]),
        BeforeDecision::Allow
    ));
}

/// 同一个键 ⇒ 执行前拒绝，拒因点名这个键。
#[test]
fn a_repeated_key_is_refused_before_execution() {
    let BeforeDecision::Refuse(refusal) =
        judgement(&json!({ "key": " k ", "value": "v" }), &[recorded("k")])
    else {
        panic!("同一个键应被拒绝");
    };
    let text = refusal.reason;
    assert!(text.contains("已经记过"), "拒因应说清是重复：{text}");
    assert!(text.contains("k"), "拒因应点名这个键：{text}");
}

/// 同一个键换一个值**仍是**重复：幂等判的是键，不是值。
#[test]
fn the_same_key_with_a_new_value_is_still_a_duplicate() {
    assert!(matches!(
        judgement(&json!({ "key": "k", "value": "other" }), &[recorded("k")]),
        BeforeDecision::Refuse(_)
    ));
}
