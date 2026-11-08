//! `behavior.rs` 的单元测试 —— 成功才记账、幂等键、无执行前拒绝。

use super::{
    ProtocolJoinCapture, ProtocolProposeCapture, PROTOCOL_JOINED_BEHAVIOR,
    PROTOCOL_PROPOSED_BEHAVIOR,
};
use crate::decl::{PROTOCOL_JOINED, PROTOCOL_PROPOSED};
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

fn input(
    tool: &str,
    success: bool,
    args: serde_json::Value,
    declared: &'static [&'static str],
) -> EffectInput {
    EffectInput {
        tool: tool.into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared,
    }
}

fn proposal_drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(
        &input(
            "protocol_propose",
            success,
            args,
            &[PROTOCOL_PROPOSED_BEHAVIOR],
        )
        .call(&env()),
    )
}

fn join_drafts(success: bool, args: serde_json::Value) -> Vec<symbio_plugin_api::FactDraft> {
    symbio_plugin_api::apply_effects(
        &input("protocol_join", success, args, &[PROTOCOL_JOINED_BEHAVIOR]).call(&env()),
    )
}

/// 一次成功提案 ⇒ 一条 `Domain × Asserted` 草稿，键带轮锚、调用 id 与协议名。
#[test]
fn a_successful_proposal_yields_one_definition_draft() {
    let d = proposal_drafts(
        true,
        json!({
            "name": "handoff",
            "title": "任务交接",
            "purpose": "把一个任务交给另一个代理",
            "messages": ["request", "done"],
        }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, PROTOCOL_PROPOSED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["name"], "handoff");
    assert_eq!(d[0].payload["title"], "任务交接");
    assert_eq!(d[0].payload["messages"], json!(["request", "done"]));
    assert_eq!(d[0].produced_by, Some(42));
    assert!(d[0].key.contains("handoff"));
}

/// 一次成功加入 ⇒ 一条 `Domain × Asserted` 草稿，键区分协议与参与者。
#[test]
fn a_successful_join_yields_one_membership_draft() {
    let d = join_drafts(
        true,
        json!({ "name": "handoff", "participant": "reviewer" }),
    );
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, PROTOCOL_JOINED);
    assert_eq!(d[0].entity, Entity::Domain);
    assert_eq!(d[0].verb, Verb::Asserted);
    assert_eq!(d[0].payload["name"], "handoff");
    assert_eq!(d[0].payload["participant"], "reviewer");
    assert_eq!(d[0].produced_by, Some(42));
    assert!(d[0].key.contains("handoff"));
    assert!(d[0].key.contains("reviewer"));
}

/// 失败调用没有效果。
#[test]
fn a_failed_call_yields_nothing() {
    assert!(proposal_drafts(
        false,
        json!({ "name": "handoff", "title": "t", "purpose": "p", "messages": ["a"] })
    )
    .is_empty());
    assert!(join_drafts(
        false,
        json!({ "name": "handoff", "participant": "reviewer" })
    )
    .is_empty());
}

/// 非法入参不产生事实（工具侧会先拒，行为不依赖它兜底）。
#[test]
fn invalid_arguments_yield_nothing() {
    for args in [
        json!({ "name": "Bad-Name", "title": "t", "purpose": "p", "messages": ["a"] }),
        json!({ "name": "handoff", "title": "  ", "purpose": "p", "messages": ["a"] }),
        json!({ "name": "handoff", "title": "t", "purpose": "p", "messages": [] }),
    ] {
        assert!(proposal_drafts(true, args).is_empty());
    }
    for args in [
        json!({ "name": "handoff", "participant": "  " }),
        json!({ "name": "Bad-Name", "participant": "reviewer" }),
        json!({ "name": "handoff", "participant": "x".repeat(65) }),
    ] {
        assert!(join_drafts(true, args).is_empty());
    }
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let i = input(
        "protocol_propose",
        true,
        json!({ "name": "handoff", "title": "t", "purpose": "p", "messages": ["a"] }),
        &[],
    );
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

/// 同一次调用重放 ⇒ 同一个幂等键；不同调用 ⇒ 各自成键（再提案 / 再加入是新证据）。
#[test]
fn replay_and_distinct_calls_get_the_expected_keys() {
    let args = json!({ "name": "handoff", "title": "t", "purpose": "p", "messages": ["a"] });
    let first = proposal_drafts(true, args.clone())[0].key.clone();
    assert_eq!(first, proposal_drafts(true, args.clone())[0].key);

    let mut second = input(
        "protocol_propose",
        true,
        args,
        &[PROTOCOL_PROPOSED_BEHAVIOR],
    );
    second.call_id = "call-2".into();
    let later = symbio_plugin_api::apply_effects(&second.call(&env()));
    assert_ne!(first, later[0].key);
}

/// 两个行为都没有 `before`：同名再提案 / 重复加入不是错误，去重由视图负责。
#[test]
fn neither_behavior_refuses_upfront() {
    use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};

    let args = json!({ "name": "handoff", "title": "t", "purpose": "p", "messages": ["a"] });
    let decision = ProtocolProposeCapture.before(&BeforeRequest {
        tool: "protocol_propose",
        call_id: "call-2",
        args: &args,
        principal: "session/a",
        declared: &[PROTOCOL_PROPOSED_BEHAVIOR],
        facts: &[],
        readings: BeforeReadings::default(),
    });
    assert!(matches!(decision, BeforeDecision::Allow));

    let args = json!({ "name": "handoff", "participant": "reviewer" });
    let decision = ProtocolJoinCapture.before(&BeforeRequest {
        tool: "protocol_join",
        call_id: "call-2",
        args: &args,
        principal: "session/a",
        declared: &[PROTOCOL_JOINED_BEHAVIOR],
        facts: &[],
        readings: BeforeReadings::default(),
    });
    assert!(matches!(decision, BeforeDecision::Allow));
}
