//! 编译贡献者的判据（DE-041 §6 / D-250 / D-262）：只编成功收束、同 trigger 至多一条、
//! 开关默认关、溯源非空。

use serde_json::json;
use symbio_plugin_api::events::{EVENT_MEMORY_ENCODED, EVENT_SESSION_CONFIG};
use symbio_plugin_api::{Contributor, Entity, Event, Seq, SettleFace, Settling, Verb};

use super::SkillCompile;
use crate::decl::SKILL_TAG;

fn ev(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    let mut e = Event::pending(
        format!("e-{seq}"),
        kind,
        Entity::Turn,
        Verb::Asserted,
        0,
        "session/main",
    );
    e.seq = Some(Seq::new(seq));
    e.ts = 1;
    e.payload = payload;
    e
}

fn enabled() -> Event {
    ev(
        1,
        EVENT_SESSION_CONFIG,
        json!({ "skill_compile_enabled": true }),
    )
}

fn settling<'a>(history: &'a [Event], response: Option<&'a str>) -> Settling<'a> {
    Settling {
        face: SettleFace::Generated,
        now_ms: 1,
        principal: "session/main",
        user_seq: 7,
        anchor: "t1",
        user_text: "把周报整理成摘要",
        response,
        recall: None,
        history,
    }
}

/// 成功收束 ⇒ 一条技能事件：正文 / 触发串 / 标签齐备，溯源指向本轮开口格。
#[test]
fn compiled_skill_requires_provenance() {
    let history = [enabled()];
    let drafts = SkillCompile.contribute(&settling(&history, Some("先列要点；再合并同类项。")));
    assert_eq!(drafts.len(), 1, "成功收束必须编出一条");
    let fact = &drafts[0].fact;
    assert_eq!(fact.kind, EVENT_MEMORY_ENCODED);
    assert_eq!(fact.payload["tag"], json!(SKILL_TAG));
    assert_eq!(fact.payload["trigger"], json!("把周报整理成摘要"));
    assert_eq!(fact.payload["content"], json!("先列要点；再合并同类项。"));
    assert_eq!(fact.payload["vis_scope"], json!("shared"));
    assert_eq!(fact.produced_by, Some(7), "技能必带溯源（I2）");
    assert_eq!(drafts[0].actor, Some(symbio_plugin_api::PRINCIPAL_USER));

    // 反向：溯源置空 ⇒ 「技能必须有溯源」那条判据要么在这条断言上失败，要么由 I2 抓住。
    assert!(fact.produced_by.is_some());
}

/// 兜底 / 中止的收束没有正文 ⇒ 一条都不编（固化失败套路比不记更坏）。
#[test]
fn only_successful_settlements_compile() {
    let history = [enabled()];
    assert!(SkillCompile
        .contribute(&settling(&history, None))
        .is_empty());
    assert!(SkillCompile
        .contribute(&settling(&history, Some("   ")))
        .is_empty());
}

/// 开关默认关：历史里没有 `session.config` 或那一格为 false ⇒ 不编。
#[test]
fn compilation_is_off_by_default() {
    assert!(SkillCompile
        .contribute(&settling(&[], Some("答")))
        .is_empty());
    let disabled = [ev(
        1,
        EVENT_SESSION_CONFIG,
        json!({ "skill_compile_enabled": false }),
    )];
    assert!(SkillCompile
        .contribute(&settling(&disabled, Some("答")))
        .is_empty());
}

/// 同一 trigger 至多一条：并发复问不追加第二条（这条判据同时兼任幂等）。
#[test]
fn same_trigger_compiles_at_most_once() {
    let previous = ev(
        2,
        EVENT_MEMORY_ENCODED,
        json!({ "tag": SKILL_TAG, "trigger": "把周报整理成摘要", "content": "旧答" }),
    );
    let history = [enabled(), previous];
    assert!(SkillCompile
        .contribute(&settling(&history, Some("新答")))
        .is_empty());
}
