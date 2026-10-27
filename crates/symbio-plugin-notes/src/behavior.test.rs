//! 效果端口的判据：按声明选实现、失败不记账、草稿带轮锚与溯源。

use super::NOTE_CAPTURE;
use crate::decl::NOTES_CREATED;
use serde_json::json;
use symbio_core::{Entity, Verb};
use symbio_plugin_api::{EffectEnv, EffectInput};

fn env() -> EffectEnv<'static> {
    EffectEnv {
        turn: 3,
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
