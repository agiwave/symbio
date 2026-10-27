//! `agent_run` 立约效果的判据：一次委托两格（立约 + 了结），没跑到的记违约。

use super::*;
use serde_json::json;
use symbio_core::Verb;
use symbio_plugin_api::effects::BeforeReadings;

fn call<'a>(args: &'a serde_json::Value, success: bool, why: &'a str) -> EffectCall<'a> {
    EffectCall {
        tool: "agent_run",
        call_id: "call-9",
        args,
        success,
        result: "子智能体已完成",
        why,
        declared: &[behaviors::DELEGATION_PROMISE],
        turn: 7,
        user_seq: 42,
        principal: "session/a",
        source: "user",
        // 立约只看「这次调用发生了什么」，不折叠既有事实：两片历史交空切片。
        snapshot: &[],
        applied: &[],
    }
}

/// 履约：立约 + 守约两格，承诺号带轮锚（跨轮不撞幂等键）。
#[test]
fn kept_promise_lands_offer_and_release() {
    let args = json!({ "prompt": "评审这份设计" });
    let drafts = DelegationPromise.after(&call(&args, true, ""));
    assert_eq!(drafts.len(), 2);

    assert_eq!(drafts[0].key, "c-offer-v2c-t7-call-9");
    assert_eq!(drafts[0].verb, Verb::Opened);
    assert_eq!(drafts[0].payload["promise"], "评审这份设计");
    assert_eq!(drafts[0].payload["from"], "session/a");
    assert_eq!(drafts[0].payload["to"], "user");
    assert_eq!(drafts[0].produced_by, Some(42));

    assert_eq!(drafts[1].key, "c-close-v2c-t7-call-9");
    assert_eq!(drafts[1].verb, Verb::Closed);
}

/// 违约：立约 + 违约 + 对协作方的宣告三格，且违约必须带原因（可观测是 T5 的前提）。
#[test]
fn broken_promise_carries_why() {
    let args = json!({ "prompt": "评审这份设计" });
    let drafts = DelegationPromise.after(&call(&args, false, "工具执行失败"));
    assert_eq!(drafts.len(), 3);
    assert_eq!(
        drafts[1].kind,
        symbio_plugin_api::events::EVENT_COMMITMENT_BROKEN
    );
    assert_eq!(drafts[1].payload["why"], "工具执行失败");
    assert_eq!(drafts[2].verb, Verb::Asserted);
}

/// 参数里没有 `prompt`（被截断成空对象）⇒ 不立约：空承诺只会让读侧把违约
/// 算在「没说过的话」上。
#[test]
fn missing_prompt_produces_nothing() {
    let args = json!({});
    assert!(DelegationPromise
        .after(&call(&args, false, "参数解析失败"))
        .is_empty());
}

/// 效果只在**声明了该行为**的调用上求值（工具侧声明是唯一开关）。
///
/// 只断言**键集合**而不是条数：本 crate 有 dev-依赖成环（测试要整棵子树），
/// 同一份 `inventory` 因此在测试二进制里出现两次（生产里不会）。条数受此影响，
/// 「选了哪些行为」不受影响。
#[test]
fn behavior_is_selected_by_declaration() {
    let args = json!({ "prompt": "评审这份设计" });
    let mut undeclared = call(&args, true, "");
    undeclared.declared = &[];
    assert!(symbio_plugin_api::apply_effects(&undeclared).is_empty());

    let declared = call(&args, true, "");
    let mut keys: Vec<String> = symbio_plugin_api::apply_effects(&declared)
        .into_iter()
        .map(|d| d.key)
        .collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys, vec!["c-close-v2c-t7-call-9", "c-offer-v2c-t7-call-9"]);
}

// ── 淘汰回路的前置判断（DE-025 D-219）────────────────────────────────

fn judgement<'a>(
    args: &'a serde_json::Value,
    principal: &'a str,
    revoked: bool,
) -> BeforeRequest<'a> {
    BeforeRequest {
        tool: "agent_run",
        call_id: "call-9",
        args,
        principal,
        declared: &[behaviors::ASSIGN_WORK],
        facts: &[],
        readings: BeforeReadings {
            assign_work_revoked: revoked,
        },
    }
}

/// 收回 `assign.work` ⇒ 拒：拒因点名主体与成因，出口是可审计拒绝（记违约承诺）。
#[test]
fn a_revoked_principal_cannot_delegate() {
    let args = json!({ "prompt": "做件事" });
    let req = judgement(&args, "session/worker", true);
    let BeforeDecision::Refuse(refusal) = DelegationGate.before(&req) else {
        panic!("已收回 assign.work 的主体必须被拒");
    };
    assert_eq!(refusal.gate, "淘汰");
    assert_eq!(refusal.kind, symbio_core::failure_kind::PERMISSION_DENIED);
    assert!(refusal.reason.contains("session/worker"), "拒因要点名主体");
    assert!(
        refusal.reason.contains("assign.work 已被淘汰回路收回"),
        "拒因要写明成因（实际: {}）",
        refusal.reason
    );
    assert!(
        refusal.notes_delegation,
        "约了没派成同样是事实：违约承诺照记"
    );
}

/// 没有生效的撤销 ⇒ 放行：判断只答减法，表是权威。
#[test]
fn an_unrevoked_principal_may_delegate() {
    let args = json!({ "prompt": "做件事" });
    assert!(matches!(
        DelegationGate.before(&judgement(&args, "session/worker", false)),
        BeforeDecision::Allow
    ));
}
