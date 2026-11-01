//! `behavior.rs` 的单元测试 —— 成功才记账、方向进幂等键、同向重复在执行前被拒。

use super::{CausalCapture, CAUSAL_LINK_BEHAVIOR};
use crate::decl::CAUSAL_LINK;
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
        tool: "causal_assert".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[CAUSAL_LINK_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功断言 ⇒ 一条 `Domain × Asserted` 草稿，幂等键带轮锚、调用 id **与方向**。
#[test]
fn a_successful_call_yields_one_assertion_draft() {
    let d = drafts(
        true,
        json!({ "cause": "内存不足", "effect": "任务超时", "confidence": 70 }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, CAUSAL_LINK);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["cause"], "内存不足");
    assert_eq!(d[0].payload["effect"], "任务超时");
    assert_eq!(d[0].payload["confidence"], 70);
    assert_eq!(d[0].produced_by, Some(42));
    assert!(
        d[0].key.contains("内存不足->任务超时"),
        "幂等键必须带方向，否则 A→B 与 B→A 会互相吞掉：{}",
        d[0].key
    );
}

/// 失败调用没有效果：一条因果断言不能被「没跑成」的调用记下。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(false, json!({ "cause": "a", "effect": "b" })).is_empty());
}

/// 自环（因 == 果）没有效果：图里多一条自环只会让每个读方都得多滤一次。
#[test]
fn a_self_loop_yields_nothing() {
    assert!(drafts(true, json!({ "cause": "a", "effect": " a " })).is_empty());
}

/// 空白侧没有效果（工具侧会先拒，但行为不依赖工具侧兜底）。
#[test]
fn a_blank_side_yields_nothing() {
    assert!(drafts(true, json!({ "cause": "  ", "effect": "b" })).is_empty());
}

/// 置信度缺省 50、上限 100：缺省值写在两处会漂移，故只有一处。
#[test]
fn confidence_defaults_and_is_capped() {
    let d = drafts(true, json!({ "cause": "a", "effect": "b" }));
    assert_eq!(d[0].payload["confidence"], 50, "缺省置信度");
    let d = drafts(
        true,
        json!({ "cause": "a", "effect": "b", "confidence": 500 }),
    );
    assert_eq!(d[0].payload["confidence"], 100, "超过 100 收进 100");
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(true, json!({ "cause": "a", "effect": "b" }));
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

fn judgement(args: &serde_json::Value, facts: &[Event]) -> BeforeDecision {
    CausalCapture.before(&BeforeRequest {
        tool: "causal_assert",
        call_id: "call-2",
        args,
        principal: "session/a",
        declared: &[CAUSAL_LINK_BEHAVIOR],
        facts,
        readings: BeforeReadings::default(),
    })
}

fn recorded(cause: &str, effect: &str) -> Event {
    Event::pending(
        "c-t3-call-0",
        CAUSAL_LINK,
        Entity::Domain,
        Verb::Asserted,
        3,
        "session/a",
    )
    .with_payload(json!({ "cause": cause, "effect": effect, "confidence": 50 }))
}

/// 会话里没有这条断言 ⇒ 放行。
#[test]
fn a_fresh_assertion_is_allowed() {
    assert!(matches!(
        judgement(
            &json!({ "cause": "a", "effect": "b" }),
            &[recorded("x", "y")]
        ),
        BeforeDecision::Allow
    ));
}

/// 同向重复 ⇒ 执行前拒绝，拒因点名这条因果关系。
#[test]
fn a_repeated_assertion_is_refused_before_execution() {
    let BeforeDecision::Refuse(refusal) = judgement(
        &json!({ "cause": " a ", "effect": "b" }),
        &[recorded("a", "b")],
    ) else {
        panic!("同向重复应被拒绝");
    };
    let text = refusal.reason;
    assert!(text.contains("已经断言过"), "拒因应说清是重复：{text}");
    assert!(text.contains("a → b"), "拒因应点名这一对：{text}");
}

/// 反向断言**不是**重复：`b → a` 是一条新信息，放行。
#[test]
fn the_opposite_direction_is_not_a_duplicate() {
    assert!(matches!(
        judgement(
            &json!({ "cause": "b", "effect": "a" }),
            &[recorded("a", "b")]
        ),
        BeforeDecision::Allow
    ));
}
