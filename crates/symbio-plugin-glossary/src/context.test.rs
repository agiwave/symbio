//! 上下文段的判据：空历史不产段、同一术语取最新释义、按首次定义序渲染、上限内截断。

use super::{GlossaryProvider, MAX_TERMS, SLOT};
use crate::decl::GLOSSARY_DEFINED;
use serde_json::json;
use symbio_plugin_api::{Cadence, ContextProvider, ContextRequest};
use symbio_plugin_api::{Entity, Event, Verb};

fn defined(id: &str, term: &str, definition: &str) -> Event {
    let mut e = Event::pending(
        id,
        GLOSSARY_DEFINED,
        Entity::Domain,
        Verb::Opened,
        "session/a",
    );
    e.payload = json!({ "term": term, "definition": definition });
    e
}

fn request<'a>(facts: &'a [Event]) -> ContextRequest<'a> {
    ContextRequest {
        request_round: 0,
        principal: "session/a",
        query: Some("债是什么"),
        facts,
        recall: None,
        live: None,
    }
}

/// 没有术语 ⇒ 一段都不产。
#[test]
fn an_empty_history_contributes_nothing() {
    let facts: Vec<Event> = Vec::new();
    assert!(GlossaryProvider.provide(&request(&facts)).is_empty());
}

/// 有术语 ⇒ 一段 `glossary_context`，正文逐条列出。
#[test]
fn terms_are_rendered_as_one_section() {
    let facts = [
        defined("e1", "债", "尚未闭合的一笔义务"),
        defined("e2", "环", "由事实驱动的执行体"),
    ];
    let sections = GlossaryProvider.provide(&request(&facts));
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].slot, SLOT);
    assert!(sections[0].text.contains("债"), "{}", sections[0].text);
    assert!(sections[0].text.contains("环"), "{}", sections[0].text);
}

/// 同一个术语定义两次 ⇒ 只渲染最新那条释义，且不重复成两条。
#[test]
fn a_redefined_term_renders_only_the_latest_definition() {
    let facts = [defined("e1", "债", "旧释义"), defined("e2", "债", "新释义")];
    let text = &GlossaryProvider.provide(&request(&facts))[0].text;
    assert!(text.contains("新释义"), "{text}");
    assert!(!text.contains("旧释义"), "{text}");
    assert_eq!(text.matches("债").count(), 1, "{text}");
}

/// 只回显最近 [`MAX_TERMS`] 条：上下文是每轮都付的成本。
#[test]
fn only_the_most_recent_terms_are_kept() {
    let facts: Vec<Event> = (0..MAX_TERMS + 2)
        .map(|i| defined(&format!("e{i}"), &format!("术语{i}"), &format!("释义{i}")))
        .collect();
    let text = &GlossaryProvider.provide(&request(&facts))[0].text;
    assert!(!text.contains("术语0："), "{text}");
    assert!(text.contains(&format!("术语{}：", MAX_TERMS + 1)), "{text}");
}

/// 收集面按 owner 找到这条 provider（同一份提交，不另写注册）。
#[test]
fn the_section_is_collected_through_the_port() {
    let facts = [defined("e1", "债", "尚未闭合的一笔义务")];
    let sections = symbio_plugin_api::context_sections(&request(&facts), Cadence::Utterance);
    let own: Vec<_> = sections.iter().filter(|s| s.slot == SLOT).collect();
    assert_eq!(own.len(), 1, "{sections:?}");
}
