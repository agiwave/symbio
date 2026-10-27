//! 效果端口的判据：选行为按声明、草稿不碰事实源、宿主面由写口补。

use super::*;
use symbio_core::{Entity, Verb};

/// 一个只为测试存在的实现：把调用记成一条 `test × asserted` 的草稿。
struct Recorder;

impl ToolBehavior for Recorder {
    fn owner(&self) -> &'static str {
        "tests/recorder"
    }

    fn behavior(&self) -> &'static str {
        crate::behaviors::TASK_TABLE
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        vec![FactDraft {
            key: format!("r-{}-{}", call.anchor(), call.call_id),
            kind: "test.recorded".to_string(),
            entity: Entity::Task,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "tool": call.tool, "ok": call.success }),
            produced_by: Some(call.user_seq),
        }]
    }
}

// 提交一条只在本测试二进制里存在的行为实现。
//
// 判据用**进程内**的提交，不依赖某个真插件被链接：本 crate 的单测里没有别的
// 插件，`inventory` 里只有这一条——「按声明选、按 owner 序」因此可以精确断言。
crate::submit_behavior!(Recorder);
fn env() -> EffectEnv {
    EffectEnv {
        turn: 3,
        user_seq: 42,
        principal: "session/a".into(),
        source: "user".into(),
    }
}

fn input(declared: &'static [&'static str]) -> EffectInput {
    EffectInput {
        tool: "local/todo_write".into(),
        call_id: "call-1".into(),
        args: serde_json::json!({}),
        success: true,
        result: "ok".into(),
        why: String::new(),
        declared,
    }
}

/// 工具没声明这个行为 ⇒ 一条草稿都不产生（「我的效果由我声明」的负向判据）。
#[test]
fn undeclared_tool_produces_nothing() {
    let i = input(&[]);
    assert!(apply(&i.call(&env())).is_empty());
}

/// 声明了就产生：草稿带轮锚、带溯源，且**不含**宿主才有的 `turn` / `actor`
/// （那两样由 `into_event` 盖章，行为无权指定）。
#[test]
fn declared_tool_produces_draft_with_anchor() {
    let i = input(&[crate::behaviors::TASK_TABLE]);
    let drafts = apply(&i.call(&env()));
    assert_eq!(drafts.len(), 1);
    let d = &drafts[0];
    assert_eq!(d.key, "r-t3-call-1");
    assert_eq!(d.produced_by, Some(42));

    let event = d.clone().into_event(7, "session/a");
    assert_eq!(event.turn, 7);
    assert_eq!(event.actor, "session/a");
    assert_eq!(event.event_id, "r-t3-call-1");
    assert_eq!(event.kind, "test.recorded");
    assert_eq!(event.entity, Entity::Task);
    assert_eq!(event.verb, Verb::Asserted);
}

/// 行为名不在 [`crate::behaviors::KNOWN`] 里 ⇒ 宿主不认识，读侧校验能抓住。
///
/// 这是 J3 的入口面：未知行为**静默不生效**是坏形态，所以实现必须自报一个已知名，
/// 而工具侧的声明由 `ToolDecls` 校验（见 loop 的 `decls.test.rs`）。
#[test]
fn test_behavior_declares_a_known_name() {
    let handlers = handlers();
    assert!(handlers.iter().any(|b| b.owner() == "tests/recorder"));
    for b in handlers {
        assert!(
            crate::behaviors::is_known(b.behavior()),
            "{} 报了未登记的行为名 {:?}",
            b.owner(),
            b.behavior()
        );
    }
}

/// `handlers()` 按 owner 字典序：装配顺序不影响落笔顺序（DE-025 §3 第 4 条）。
#[test]
fn handlers_are_sorted_by_owner() {
    let owners: Vec<&str> = handlers().iter().map(|b| b.owner()).collect();
    let mut sorted = owners.clone();
    sorted.sort_unstable();
    assert_eq!(owners, sorted);
}
