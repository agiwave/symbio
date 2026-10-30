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
            key: format!("r-{}-{}", call.anchor, call.call_id),
            kind: "test.recorded".to_string(),
            entity: Entity::Task,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "tool": call.tool, "ok": call.success }),
            produced_by: Some(call.user_seq),
        }]
    }

    fn annotate(&self, input: &EffectInput) -> Vec<(&'static str, serde_json::Value)> {
        vec![("test_tool", serde_json::Value::String(input.tool.clone()))]
    }
}

// 提交一条只在本测试二进制里存在的行为实现。
//
// 判据用**进程内**的提交，不依赖某个真插件被链接：本 crate 的单测里没有别的
// 插件，`inventory` 里只有这一条——「按声明选、按 owner 序」因此可以精确断言。
crate::submit_behavior!(Recorder);

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

/// 声明了就产生：草稿带词干、带溯源，而信封（轮号 / 归属 / 时刻）不在草稿里——
/// 行为无权指定「这笔账记在哪一轮」，那是写口的事（DE-026 D-252）。
#[test]
fn declared_tool_produces_draft_with_anchor() {
    let i = input(&[crate::behaviors::TASK_TABLE]);
    let drafts = apply(&i.call(&env()));
    assert_eq!(drafts.len(), 1);
    let d = &drafts[0];
    assert_eq!(d.key, "r-t3-call-1");
    assert_eq!(d.produced_by, Some(42));

    let event = d.clone().into_event("session/a");
    assert_eq!(event.turn, 0, "草稿不带轮号：信封由写口盖（D-252）");
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

/// 富化与效果走同一套选法：没声明就没有补充字段，声明了才补。
///
/// 这条判据把 `annotate` 钉在**声明**上而不是名字上——文件变更那一路能迁出 core，
/// 靠的就是这一点（D-214）。
#[test]
fn annotations_follow_the_declaration() {
    let undeclared = input(&[]);
    assert!(annotations(&undeclared).is_empty());

    let declared = input(&[crate::behaviors::TASK_TABLE]);
    let got = annotations(&declared);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "test_tool");
    assert_eq!(got[0].1, serde_json::json!("local/todo_write"));
}

/// 富化只需要「发生了什么」，因此**不**要求宿主面：调用收口就能算（D-214 第 1 条）。
///
/// 判据形态：`annotations` 的入参是 `EffectInput`，编译期就交不出 `turn` / `principal`。
#[test]
fn annotations_do_not_need_the_host_face() {
    let i = input(&[crate::behaviors::TASK_TABLE]);
    // 若富化需要轮锚，这里必须构造 `EffectEnv`——它构造不出来就说明时机确实更早。
    assert_eq!(annotations(&i).len(), 1);
}

/// `facts()` 按发生顺序串两片历史：轮初的在前，本轮刚落格的在后。
///
/// 这是 D-214 第 2 条的存在理由——同轮两个效果要互相看得见，否则第二条声明会把
/// 第一条刚开的格再开一次。
#[test]
fn facts_chain_snapshot_then_applied() {
    let old = Event::pending("e1", "test.recorded", Entity::Task, Verb::Asserted, 1, "a");
    let fresh = Event::pending("e2", "test.recorded", Entity::Task, Verb::Asserted, 3, "a");
    let snapshot = [old];
    let applied = [fresh];
    let env = EffectEnv {
        anchor: "t3",
        user_seq: 42,
        principal: "session/a",
        source: "user",
        snapshot: &snapshot,
        applied: &applied,
    };
    let ids: Vec<&str> = env.facts().map(|e| e.event_id.as_str()).collect();
    assert_eq!(ids, ["e1", "e2"]);
}

// ── 前置判断（D-219）────────────────────────────────────────────────

/// 一个只管前置判断的实现：读数里「已收回」⇒ 拒绝；同时验证它**不造格**。
struct Gatekeeper;

impl ToolBehavior for Gatekeeper {
    fn owner(&self) -> &'static str {
        "tests/gatekeeper"
    }

    fn behavior(&self) -> &'static str {
        crate::behaviors::NOTE_CAPTURE
    }

    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        if !req.readings.assign_work_revoked {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(
            BeforeRefusal::new(
                "测试闸",
                format!("{} 已被收回", req.principal),
                "permission_denied",
            )
            .without_delegation_note(),
        )
    }

    fn after(&self, _call: &EffectCall<'_>) -> Vec<FactDraft> {
        panic!("前置判断的实现不该在这次判据里被问 after");
    }
}

crate::submit_behavior!(Gatekeeper);

fn before_req<'a>(
    declared: &'static [&'static str],
    args: &'a Value,
    facts: &'a [Event],
    revoked: bool,
) -> BeforeRequest<'a> {
    BeforeRequest {
        tool: "local/notes_add",
        call_id: "call-9",
        args,
        principal: "session/a",
        declared,
        facts,
        readings: BeforeReadings {
            assign_work_revoked: revoked,
        },
    }
}

/// 没声明就没有判断：宿主不问、实现也不会被调到（「我的这一关由我声明」的负向判据）。
#[test]
fn undeclared_tools_are_not_judged() {
    let args = serde_json::json!({});
    let req = before_req(&[], &args, &[], true);
    assert!(apply_before(&req).is_none());
}

/// 中性读数 ⇒ 放行：判断只在命中自己的条件时拒，不因「表里还有别的实现」而误伤。
#[test]
fn neutral_readings_pass() {
    let args = serde_json::json!({});
    let req = before_req(&[crate::behaviors::NOTE_CAPTURE], &args, &[], false);
    assert!(apply_before(&req).is_none());
}

/// 命中 ⇒ 拒绝：拒因带名字与主体，退出语义由实现自己声明（这里不记承诺）。
#[test]
fn a_declared_judgement_refuses_with_its_own_semantics() {
    let args = serde_json::json!({});
    let req = before_req(&[crate::behaviors::NOTE_CAPTURE], &args, &[], true);
    let refusal = apply_before(&req).expect("已收回必须被拒");
    assert_eq!(refusal.gate, "测试闸");
    assert_eq!(refusal.reason, "session/a 已被收回");
    assert!(!refusal.notes_delegation);
}
