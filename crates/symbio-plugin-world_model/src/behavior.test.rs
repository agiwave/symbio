//! `behavior.rs` 的单元测试 —— 成功才记账、幂等键带被观察的键、后续观察不被拦。

use super::{WorldCapture, WORLD_OBSERVED_BEHAVIOR};
use crate::decl::WORLD_OBSERVED;
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
        tool: "world_observe".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[WORLD_OBSERVED_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功观察 ⇒ 一条 `Domain × Asserted` 草稿，幂等键带轮锚、调用 id 与被观察的键。
#[test]
fn a_successful_call_yields_one_observation_draft() {
    let d = drafts(
        true,
        json!({ "entity": "服务器", "attribute": "状态", "value": "离线", "confidence": 80 }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, WORLD_OBSERVED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["entity"], "服务器");
    assert_eq!(d[0].payload["attribute"], "状态");
    assert_eq!(d[0].payload["value"], "离线");
    assert_eq!(d[0].payload["confidence"], 80);
    assert_eq!(d[0].produced_by, Some(42));
    assert!(
        d[0].key.contains("服务器.状态"),
        "幂等键应点名被观察的键，便于在转写里定位：{}",
        d[0].key
    );
}

/// 失败调用没有效果：一条世界状态不能被「没跑成」的调用记下。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(
        false,
        json!({ "entity": "a", "attribute": "b", "value": "c" })
    )
    .is_empty());
}

/// 空值没有效果（工具侧会先拒，但行为不依赖工具侧兜底）。
#[test]
fn a_blank_value_yields_nothing() {
    assert!(drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "  " })
    )
    .is_empty());
}

/// 置信度缺省 50、上限 100：缺省值写在两处会漂移，故只有一处。
#[test]
fn confidence_defaults_and_is_capped() {
    let d = drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    assert_eq!(d[0].payload["confidence"], 50, "缺省置信度");
    let d = drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c", "confidence": 500 }),
    );
    assert_eq!(d[0].payload["confidence"], 100, "超过 100 收进 100");
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

/// 同一次调用重放 ⇒ 同一个幂等键（不会把一条观察记两次）。
#[test]
fn a_replayed_call_keeps_the_same_key() {
    let a = drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    let b = drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    assert_eq!(a[0].key, b[0].key);
}

/// 不同调用 ⇒ 不同幂等键：世界会变，后来的观察必须能落成新证据。
#[test]
fn a_different_call_gets_its_own_key() {
    let mut second = input(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    second.call_id = "call-2".into();
    let first = drafts(
        true,
        json!({ "entity": "a", "attribute": "b", "value": "c" }),
    );
    let later = symbio_plugin_api::apply_effects(&second.call(&env()));
    assert_ne!(first[0].key, later[0].key);
}

/// 本行为不设 `before`：同一实体同一属性再观察一次不是错误，是它变了的证据。
#[test]
fn a_repeat_observation_is_not_refused() {
    use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};
    let decision = WorldCapture.before(&BeforeRequest {
        tool: "world_observe",
        call_id: "call-2",
        args: &json!({ "entity": "a", "attribute": "b", "value": "c" }),
        principal: "session/a",
        declared: &[WORLD_OBSERVED_BEHAVIOR],
        facts: &[],
        readings: BeforeReadings::default(),
    });
    assert!(matches!(decision, BeforeDecision::Allow));
}
