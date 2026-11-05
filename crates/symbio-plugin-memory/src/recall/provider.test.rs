//! 召回段的 provider：段身份、空态、节奏、提交。
//!
//! 排版细节不在这里判（[`super::section`] 的用例钉）：这里只断言「provider 在什么
//! 情况下给段、给的哪一段」。这几条原来住 `loop` 的 `context/providers.test.rs`，
//! 随 provider 一起搬来（DE-026 D-264）。

use super::*;
use symbio_plugin_api::context::{slots, Cadence, RecallFacts, RecallLine};
use symbio_plugin_api::ContextProvider;

fn request<'a>(recall: Option<&'a RecallFacts<'a>>) -> ContextRequest<'a> {
    ContextRequest {
        request_round: 0,
        principal: "session/main",
        query: Some("上次那份设计"),
        facts: &[],
        recall,
        live: None,
    }
}

/// 控制面没给读数 ⇒ 一段都不给（整段省略，不印空壳标题）。
#[test]
fn without_a_reading_contributes_nothing() {
    assert!(RecallProvider.provide(&request(None)).is_empty());
}

/// 给了读数 ⇒ 交出 `slots::RECALL`，正文里带着原文与标签。
#[test]
fn renders_the_judged_reading() {
    let lines = [RecallLine {
        seq: 1,
        content: "把总结写进 tmp/a.md",
        tag: "经验",
    }];
    let recall = RecallFacts {
        degraded: false,
        lines: &lines,
    };
    let got = RecallProvider.provide(&request(Some(&recall)));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].slot, slots::RECALL);
    assert!(
        got[0].text.contains("把总结写进 tmp/a.md"),
        "正文要带原文：{}",
        got[0].text
    );
    assert!(got[0].text.contains("经验"), "正文要带认知标签");
}

/// 轮首那一格读数（`session.recall` 事实）在场 ⇒ 渲染它。
///
/// 会话插件在轮首把收放后的切片交给 `retrieve_all`，把读数落成事实；provider
/// 拿不到会话存储，只能从 `req.facts` 按名字读这一格（本用例钉住这条通路）。
#[test]
fn renders_the_turn_opening_reading_from_facts() {
    let mut fact = symbio_plugin_api::Event::pending(
        "session-recall:u1",
        EVENT_SESSION_RECALL,
        symbio_plugin_api::Entity::Scope,
        symbio_plugin_api::Verb::Asserted,
        "session/main",
    );
    fact.payload = serde_json::json!({
        "degraded": false,
        "entries": [{ "seq": 7, "content": "把总结写进 tmp/a.md", "tag": "经验" }],
    });
    let req = ContextRequest {
        request_round: 0,
        principal: "session/main",
        query: Some("上次那份设计"),
        facts: &[fact],
        recall: None,
        live: None,
    };
    let got = RecallProvider.provide(&req);
    assert_eq!(got.len(), 1, "读数在场 ⇒ 交出召回段");
    assert_eq!(got[0].slot, slots::RECALL);
    assert!(got[0].text.contains("把总结写进 tmp/a.md"), "{}", got[0].text);
    assert!(got[0].text.contains("经验"), "正文要带认知标签");
}

/// 发言级节奏（召回在任何段渲染之前取到），且提交在本包名下。
///
/// owner 决定段的次序（DE-025 §3 第 4 条）：`loop` < `memory` < `task` ⇒ 发言级仍是
/// 「召回 → 调度」，与召回段住 `loop` 时逐字相同。
#[test]
fn is_submitted_under_memory_at_utterance_cadence() {
    let submitted: Vec<&'static str> = symbio_plugin_api::context_providers()
        .iter()
        .filter(|p| p.cadence() == Cadence::Utterance)
        .map(|p| p.owner())
        .collect();
    assert!(
        submitted.contains(&"memory"),
        "召回段必须经 submit_context_provider! 提交（否则模型看不到过往记忆）：{submitted:?}"
    );
    assert_eq!(RecallProvider.cadence(), Cadence::Utterance);
}
