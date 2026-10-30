//! 扩展面的判据：按声明选实现、失败不记账、草稿带轮锚与溯源、重复便签在执行前被拒。

use super::{NoteCapture, NOTE_CAPTURE};
use crate::decl::NOTES_CREATED;
use serde_json::json;
use symbio_plugin_api::effects::{BeforeDecision, BeforeReadings, BeforeRequest};
use symbio_plugin_api::{EffectEnv, EffectInput, ToolBehavior};
use symbio_plugin_api::{Entity, Event, Verb};

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
        tool: "notes_add".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[NOTE_CAPTURE],
    }
}

/// 一次成功调用 ⇒ 一条 `Domain × Opened` 草稿，幂等键带轮锚与调用 id。
#[test]
fn a_successful_call_yields_one_domain_draft() {
    let drafts =
        symbio_plugin_api::apply_effects(&input(true, json!({ "text": "买牛奶" })).call(&env()));
    assert_eq!(drafts.len(), 1);
    let d = &drafts[0];
    assert_eq!(d.key, "n-t3-call-1");
    assert_eq!(d.kind, NOTES_CREATED);
    assert_eq!(d.entity, Entity::Domain);
    assert_eq!(d.verb, Verb::Opened);
    assert_eq!(d.payload, json!({ "text": "买牛奶" }));
    assert_eq!(d.produced_by, Some(42));
}

/// 失败调用没有效果：一条便签不能被「没跑成」的调用记下。
#[test]
fn a_failed_call_yields_nothing() {
    let drafts =
        symbio_plugin_api::apply_effects(&input(false, json!({ "text": "买牛奶" })).call(&env()));
    assert!(drafts.is_empty());
}

/// 空白正文没有效果（工具侧会先拒，但行为不依赖工具侧兜底）。
#[test]
fn a_blank_note_yields_nothing() {
    let drafts =
        symbio_plugin_api::apply_effects(&input(true, json!({ "text": "   " })).call(&env()));
    assert!(drafts.is_empty());
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生（「效果由我声明」的负向判据）。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let mut i = input(true, json!({ "text": "买牛奶" }));
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

fn judgement(args: &serde_json::Value, facts: &[Event]) -> BeforeDecision {
    NoteCapture.before(&BeforeRequest {
        tool: "notes_add",
        call_id: "call-2",
        args,
        principal: "session/a",
        declared: &[NOTE_CAPTURE],
        facts,
        readings: BeforeReadings::default(),
    })
}

fn recorded(text: &str) -> Event {
    Event::pending(
        "n-t3-call-1",
        NOTES_CREATED,
        Entity::Domain,
        Verb::Opened,
        3,
        "session/a",
    )
    .with_payload(json!({ "text": text }))
}

/// 会话里没有这条便签 ⇒ 放行。
#[test]
fn a_fresh_note_is_allowed() {
    let args = json!({ "text": "买牛奶" });
    assert!(matches!(
        judgement(&args, &[recorded("交报告")]),
        BeforeDecision::Allow
    ));
}

/// 同一会话已记过同一条（只差首尾空白）⇒ 执行前拒绝，拒因点名重复正文。
#[test]
fn a_repeated_note_is_refused_before_execution() {
    let args = json!({ "text": " 买牛奶 " });
    let BeforeDecision::Refuse(refusal) = judgement(&args, &[recorded("买牛奶")]) else {
        panic!("重复便签应在执行前被拒");
    };
    assert_eq!(refusal.gate, "便签");
    assert_eq!(refusal.kind, symbio_plugin_api::failure_kind::ERROR);
    assert!(
        refusal.reason.contains("买牛奶"),
        "拒因应带回重复正文: {}",
        refusal.reason
    );
    assert!(refusal.notes_delegation);
}

/// 别的领域事实不算数：只认本插件自己的 `notes.created`。
#[test]
fn a_fact_of_another_kind_does_not_count_as_a_note() {
    let args = json!({ "text": "买牛奶" });
    let other = Event::pending(
        "x-1",
        "chat.assistant.final",
        Entity::Turn,
        Verb::Asserted,
        3,
        "session/a",
    )
    .with_payload(json!({ "text": "买牛奶" }));
    assert!(matches!(judgement(&args, &[other]), BeforeDecision::Allow));
}
