//! 上下文端口的判据：空历史不产段、有便签则按最近几条渲染。

use super::{NotesProvider, MAX_NOTES, SLOT};
use crate::decl::NOTES_CREATED;
use serde_json::json;
use symbio_core::{Entity, Event, Verb};
use symbio_plugin_api::{Cadence, ContextProvider, ContextRequest};

fn note(id: &str, text: &str) -> Event {
    let mut e = Event::pending(
        id,
        NOTES_CREATED,
        Entity::Domain,
        Verb::Opened,
        1,
        "session/a",
    );
    e.payload = json!({ "text": text });
    e
}

fn request<'a>(facts: &'a [Event]) -> ContextRequest<'a> {
    ContextRequest {
        turn: 2,
        request_round: 0,
        principal: "session/a",
        query: Some("我记了什么"),
        facts,
        recall: None,
        task_cap: 8,
        live: None,
    }
}

/// 没有便签 ⇒ 一段都不产（默认形态：多数来源在多数轮次上为空）。
#[test]
fn an_empty_history_contributes_nothing() {
    let facts: Vec<Event> = Vec::new();
    assert!(NotesProvider.provide(&request(&facts)).is_empty());
}

/// 有便签 ⇒ 一段 `notes_context`，正文逐条列出。
#[test]
fn notes_are_rendered_as_one_section() {
    let facts = [note("e1", "买牛奶"), note("e2", "周五交周报")];
    let sections = NotesProvider.provide(&request(&facts));
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].slot, SLOT);
    assert!(sections[0].text.contains("买牛奶"), "{}", sections[0].text);
    assert!(
        sections[0].text.contains("周五交周报"),
        "{}",
        sections[0].text
    );
}

/// 只回显最近 [`MAX_NOTES`] 条：上下文是每轮都付的成本。
#[test]
fn only_the_most_recent_notes_are_kept() {
    let facts: Vec<Event> = (0..MAX_NOTES + 2)
        .map(|i| note(&format!("e{i}"), &format!("第 {i} 条")))
        .collect();
    let sections = NotesProvider.provide(&request(&facts));
    assert_eq!(sections.len(), 1);
    let text = &sections[0].text;
    assert!(!text.contains("第 0 条"), "{text}");
    assert!(text.contains(&format!("第 {} 条", MAX_NOTES + 1)), "{text}");
}

/// 收集面按 owner 找到这条 provider（同一份提交，不另写注册）。
#[test]
fn the_section_is_collected_through_the_port() {
    let facts = [note("e1", "买牛奶")];
    let sections = symbio_plugin_api::context_sections(&request(&facts), Cadence::Turn);
    let own: Vec<_> = sections.iter().filter(|s| s.slot == SLOT).collect();
    assert_eq!(own.len(), 1, "{sections:?}");
}
