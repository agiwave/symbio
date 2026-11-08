//! `behavior.rs` 的单元测试 —— 成功才记账、owner 是常量、修订不被拦。

use super::{ToolsmithCapture, TOOLSMITH_DEFINED_BEHAVIOR};
use crate::decl::{OWNER_PLUGIN, TOOLSMITH_DEFINED};
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
        tool: "toolsmith_define".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[TOOLSMITH_DEFINED_BEHAVIOR],
    }
}

fn drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(&input(success, args).call(&env()))
}

/// 一次成功登记 ⇒ 一条 `Domain × Asserted` 草稿，幂等键带轮锚、调用 id 与工具名。
#[test]
fn a_successful_call_yields_one_definition_draft() {
    let d = drafts(
        true,
        json!({ "name": "summarize_ticket", "description": "压成一句话", "input_schema": { "type": "object" } }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, TOOLSMITH_DEFINED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["name"], "summarize_ticket");
    assert_eq!(d[0].payload["description"], "压成一句话");
    assert_eq!(d[0].payload["input_schema"]["type"], "object");
    assert_eq!(d[0].payload["owner"], OWNER_PLUGIN, "owner 是常量");
    assert_eq!(d[0].produced_by, Some(42));
    assert!(d[0].key.contains("summarize_ticket"));
}

/// **受限判据**：调用方夹带 `owner: "vdfs"`，落格的 owner 仍是 `toolsmith`。
#[test]
fn a_smuggled_owner_does_not_change_the_draft() {
    let d = drafts(
        true,
        json!({
            "name": "summarize_ticket",
            "description": "d",
            "input_schema": {},
            "owner": "vdfs",
        }),
    );
    assert_eq!(d[0].payload["owner"], "toolsmith");
}

/// 失败调用没有效果。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(drafts(
        false,
        json!({ "name": "good_name", "description": "d", "input_schema": {} })
    )
    .is_empty());
}

/// 名字不合法或 schema 不是对象时不产生事实（工具侧会先拒，行为不依赖它兜底）。
#[test]
fn an_invalid_definition_yields_nothing() {
    for args in [
        json!({ "name": "Bad-Name", "description": "d", "input_schema": {} }),
        json!({ "name": "good_name", "description": "  ", "input_schema": {} }),
        json!({ "name": "good_name", "description": "d", "input_schema": "x" }),
    ] {
        assert!(drafts(true, args).is_empty());
    }
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(
        true,
        json!({ "name": "good_name", "description": "d", "input_schema": {} }),
    );
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

/// 同一次调用重放 ⇒ 同一个幂等键。
#[test]
fn a_replayed_call_keeps_the_same_key() {
    let args = json!({ "name": "good_name", "description": "d", "input_schema": {} });
    assert_eq!(drafts(true, args.clone())[0].key, drafts(true, args)[0].key);
}

/// 不同调用 ⇒ 不同幂等键：定义可以修订，后一次必须能落成新证据。
#[test]
fn a_different_call_gets_its_own_key() {
    let args = json!({ "name": "good_name", "description": "d", "input_schema": {} });
    let first = drafts(true, args.clone());
    let mut second = input(true, args);
    second.call_id = "call-2".into();
    let later = symbio_plugin_api::apply_effects(&second.call(&env()));
    assert_ne!(first[0].key, later[0].key);
}

/// 本行为不设 `before`：同名再登记是修订，不是错误。
#[test]
fn a_redefinition_is_not_refused() {
    use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};
    let args = json!({ "name": "good_name", "description": "新说明", "input_schema": {} });
    let decision = ToolsmithCapture.before(&BeforeRequest {
        tool: "toolsmith_define",
        call_id: "call-2",
        args: &args,
        principal: "session/a",
        declared: &[TOOLSMITH_DEFINED_BEHAVIOR],
        facts: &[],
        readings: BeforeReadings::default(),
    });
    assert!(matches!(decision, BeforeDecision::Allow));
}
