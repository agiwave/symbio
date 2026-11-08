//! `behavior.rs` 的单元测试 —— 成功才记账、幂等键带被标注的动作、修正判断不被拦。

use super::{ReversibilityCapture, REVERSIBILITY_ASSESSED_BEHAVIOR};
use crate::decl::REVERSIBILITY_ASSESSED;
use serde_json::json;
use symbio_plugin_api::{EffectEnv, EffectInput, Entity, ToolBehavior, Verb};

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
        tool: "reversibility_annotate".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[REVERSIBILITY_ASSESSED_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功标注 ⇒ 一条 `Domain × Asserted` 草稿，幂等键带轮锚、调用 id 与被标注的动作。
#[test]
fn a_successful_call_yields_one_assessment_draft() {
    let d = drafts(
        true,
        json!({ "action": "改配置", "level": "compensable", "undo": "把旧配置写回" }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, REVERSIBILITY_ASSESSED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["action"], "改配置");
    assert_eq!(d[0].payload["level"], "compensable");
    assert_eq!(d[0].payload["undo"], "把旧配置写回");
    assert_eq!(d[0].produced_by, Some(42));
    assert!(
        d[0].key.contains("改配置"),
        "幂等键应点名被标注的动作：{}",
        d[0].key
    );
}

/// 失败调用没有效果。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(false, json!({ "action": "x", "level": "reversible" })).is_empty());
}

/// 没有补偿做法的 `compensable` 不产生事实（工具侧会先拒，行为不依赖它兜底）。
#[test]
fn compensable_without_undo_yields_nothing() {
    assert!(drafts(true, json!({ "action": "x", "level": "compensable" })).is_empty());
}

/// 不认识的等级不产生事实。
#[test]
fn an_unknown_level_yields_nothing() {
    assert!(drafts(true, json!({ "action": "x", "level": "maybe" })).is_empty());
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(true, json!({ "action": "x", "level": "reversible" }));
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

/// 同一次调用重放 ⇒ 同一个幂等键。
#[test]
fn a_replayed_call_keeps_the_same_key() {
    let a = drafts(true, json!({ "action": "x", "level": "reversible" }));
    let b = drafts(true, json!({ "action": "x", "level": "reversible" }));
    assert_eq!(a[0].key, b[0].key);
}

/// 不同调用 ⇒ 不同幂等键：判断可以修正，后来的判断必须能落成新证据。
#[test]
fn a_different_call_gets_its_own_key() {
    let mut second = input(true, json!({ "action": "x", "level": "reversible" }));
    second.call_id = "call-2".into();
    let first = drafts(true, json!({ "action": "x", "level": "reversible" }));
    let later = symbio_plugin_api::apply_effects(&second.call(&env()));
    assert_ne!(first[0].key, later[0].key);
}

/// 本行为不设 `before`：同一动作再标注一次是修正判断，不是错误。
#[test]
fn a_reassessment_is_not_refused() {
    use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};
    let decision = ReversibilityCapture.before(&BeforeRequest {
        tool: "reversibility_annotate",
        call_id: "call-2",
        args: &json!({ "action": "x", "level": "irreversible" }),
        principal: "session/a",
        declared: &[REVERSIBILITY_ASSESSED_BEHAVIOR],
        facts: &[],
        readings: BeforeReadings::default(),
    });
    assert!(matches!(decision, BeforeDecision::Allow));
}
