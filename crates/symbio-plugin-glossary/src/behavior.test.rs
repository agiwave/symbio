//! 扩展面的判据：按声明选实现、失败不记账、草稿带轮锚与溯源、重复释义在执行前被拒。

use super::{GlossaryRecord, GLOSSARY_RECORD};
use crate::decl::GLOSSARY_DEFINED;
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
        tool: "glossary_define".into(),
        call_id: "call-1".into(),
        args,
        success,
        result: "ok".into(),
        why: String::new(),
        declared: &[GLOSSARY_RECORD],
    }
}

/// 一次成功调用 ⇒ 一条 `Domain × Opened` 草稿，幂等键带轮锚与调用 id。
#[test]
fn a_successful_call_yields_one_domain_draft() {
    let args = json!({ "term": "债", "definition": "尚未闭合的一笔义务" });
    let drafts = symbio_plugin_api::apply_effects(&input(true, args).call(&env()));
    assert_eq!(drafts.len(), 1);
    let d = &drafts[0];
    assert_eq!(d.key, "g-t3-call-1");
    assert_eq!(d.kind, GLOSSARY_DEFINED);
    assert_eq!(d.entity, Entity::Domain);
    assert_eq!(d.verb, Verb::Opened);
    assert_eq!(
        d.payload,
        json!({ "term": "债", "definition": "尚未闭合的一笔义务" })
    );
    assert_eq!(d.produced_by, Some(42));
}

/// 失败调用没有效果。
#[test]
fn a_failed_call_yields_nothing() {
    let args = json!({ "term": "债", "definition": "尚未闭合的一笔义务" });
    let drafts = symbio_plugin_api::apply_effects(&input(false, args).call(&env()));
    assert!(drafts.is_empty());
}

/// 空白术语没有效果（工具侧会先拒，行为不依赖工具侧兜底）。
#[test]
fn a_blank_term_yields_nothing() {
    let args = json!({ "term": "   ", "definition": "尚未闭合的一笔义务" });
    let drafts = symbio_plugin_api::apply_effects(&input(true, args).call(&env()));
    assert!(drafts.is_empty());
}

/// 工具没声明这个行为名 ⇒ 一条草稿都不产生。
#[test]
fn an_undeclared_tool_does_not_reach_the_behavior() {
    let args = json!({ "term": "债", "definition": "尚未闭合的一笔义务" });
    let mut i = input(true, args);
    i.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&i.call(&env())).is_empty());
}

fn judgement(args: &serde_json::Value, facts: &[Event]) -> BeforeDecision {
    GlossaryRecord.before(&BeforeRequest {
        tool: "glossary_define",
        call_id: "call-2",
        args,
        principal: "session/a",
        declared: &[GLOSSARY_RECORD],
        facts,
        readings: BeforeReadings::default(),
    })
}

fn recorded(term: &str, definition: &str) -> Event {
    Event::pending(
        "g-t3-call-1",
        GLOSSARY_DEFINED,
        Entity::Domain,
        Verb::Opened,
        "session/a",
    )
    .with_payload(json!({ "term": term, "definition": definition }))
}

/// 会话里还没有这个术语 ⇒ 放行。
#[test]
fn a_fresh_term_is_allowed() {
    let args = json!({ "term": "环", "definition": "由事实驱动的执行体" });
    assert!(matches!(
        judgement(&args, &[recorded("债", "尚未闭合的一笔义务")]),
        BeforeDecision::Allow
    ));
}

/// 同一个术语换一条释义 ⇒ 放行（术语表允许覆盖）。
#[test]
fn a_redefinition_with_a_new_definition_is_allowed() {
    let args = json!({ "term": "债", "definition": "换一种说法" });
    assert!(matches!(
        judgement(&args, &[recorded("债", "尚未闭合的一笔义务")]),
        BeforeDecision::Allow
    ));
}

/// 同一个术语 + 同样的释义（只差首尾空白）⇒ 执行前拒绝。
#[test]
fn a_repeated_definition_is_refused_before_execution() {
    let args = json!({ "term": " 债 ", "definition": " 尚未闭合的一笔义务 " });
    let BeforeDecision::Refuse(refusal) = judgement(&args, &[recorded("债", "尚未闭合的一笔义务")])
    else {
        panic!("重复释义应在执行前被拒");
    };
    assert_eq!(refusal.gate, "术语");
    assert_eq!(refusal.kind, symbio_plugin_api::failure_kind::ERROR);
    assert!(
        refusal.reason.contains('债'),
        "拒因应带回术语名: {}",
        refusal.reason
    );
    assert!(refusal.notes_delegation);
}

/// 别的领域事实不算数：只认本插件自己的 `glossary.defined`。
#[test]
fn a_fact_of_another_kind_does_not_count_as_a_definition() {
    let args = json!({ "term": "债", "definition": "尚未闭合的一笔义务" });
    let other = Event::pending(
        "x-1",
        "notes.created",
        Entity::Domain,
        Verb::Opened,
        "session/a",
    )
    .with_payload(json!({ "term": "债", "definition": "尚未闭合的一笔义务" }));
    assert!(matches!(judgement(&args, &[other]), BeforeDecision::Allow));
}
