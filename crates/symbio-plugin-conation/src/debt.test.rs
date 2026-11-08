//! 升格债的三个 early return（`D-386`）：开关、授权、已声明。
//!
//! 授权那一格在一个正常装配里恒真（`PRINCIPAL_AUTONOMOUS` 持 `define.work`），所以
//! 这里没有「拒绝」用例——它由 `production_matrix` 的授权表保证，本包不造一个假矩阵
//! 来演它。开关、宽度、已声明这三条才是本包自己的判据。

use serde_json::{json, Value};

use symbio_plugin_api::debt::{DebtFacts, DebtSource, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CONATION_EXPRESSED, EVENT_SESSION_CONFIG, EVENT_TASK_OPENED,
};

use super::{PromoteDebts, MAX_GOAL_LEN, PROMOTE_DEBT, PROMOTE_PORT};

fn request() -> DriveRequest {
    DriveRequest {
        session_id: "s-1".to_string(),
        provider: Some("main".to_string()),
        principal: "autonomous".to_string(),
    }
}

fn config(facts: &mut DebtFacts, id: &str, enabled: Value) {
    facts.note_fact(EVENT_SESSION_CONFIG, id);
    facts.note_fact_payload(
        EVENT_SESSION_CONFIG,
        id,
        json!({ "conation_enabled": enabled }),
    );
}

fn expressed(facts: &mut DebtFacts, id: &str, goal: &str, source_seq: u64) {
    facts.note_fact(EVENT_CONATION_EXPRESSED, id);
    facts.note_fact_payload(
        EVENT_CONATION_EXPRESSED,
        id,
        json!({ "goal": goal, "source_seq": source_seq }),
    );
}

fn opened(facts: &mut DebtFacts, id: &str, goal: &str) {
    facts.note_fact(EVENT_TASK_OPENED, id);
    facts.note_fact_payload(EVENT_TASK_OPENED, id, json!({ "goal": goal }));
}

fn rows(facts: &DebtFacts) -> Vec<symbio_plugin_api::debt::DebtSpec> {
    PromoteDebts.rows(&request(), facts)
}

#[test]
fn the_switch_off_produces_no_debt() {
    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(false));
    expressed(&mut facts, "want-1", "把周报写了", 3);
    assert!(rows(&facts).is_empty());
}

#[test]
fn the_switch_reads_the_latest_config_in_this_round() {
    let mut facts = DebtFacts::default();
    // 上一轮关着、本轮开着 ⇒ 本轮开口径（`D-386`）：最近一份为准。
    config(&mut facts, "cfg-1", json!(false));
    config(&mut facts, "cfg-2", json!(true));
    expressed(&mut facts, "want-1", "把周报写了", 3);
    assert_eq!(rows(&facts).len(), 1, "本轮已开 ⇒ 欠这笔债");

    // 反过来：上一轮开着、本轮关 ⇒ 不欠（不等下一轮）。
    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    config(&mut facts, "cfg-2", json!(false));
    expressed(&mut facts, "want-1", "把周报写了", 3);
    assert!(rows(&facts).is_empty());
}

#[test]
fn a_missing_or_non_boolean_switch_is_off() {
    let mut facts = DebtFacts::default();
    expressed(&mut facts, "want-1", "看一眼", 3);
    assert!(rows(&facts).is_empty(), "没有 config ⇒ 关");

    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!("true"));
    expressed(&mut facts, "want-1", "看一眼", 3);
    assert!(rows(&facts).is_empty(), "字符串不是布尔 ⇒ 关");
}

#[test]
fn the_width_gate_rejects_only_past_the_owner_limit() {
    let at_limit = "x".repeat(MAX_GOAL_LEN);
    let over = "x".repeat(MAX_GOAL_LEN + 1);

    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    expressed(&mut facts, "want-1", &at_limit, 3);
    assert_eq!(rows(&facts).len(), 1, "正好等于上界是允许的");

    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    expressed(&mut facts, "want-1", &over, 3);
    assert!(rows(&facts).is_empty(), "超过上界不升格");
}

#[test]
fn a_goal_already_declared_is_not_owed_again() {
    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    expressed(&mut facts, "want-1", "把周报写了", 3);
    opened(&mut facts, "task-1", "把周报写了");
    assert!(rows(&facts).is_empty());
}

#[test]
fn each_live_intent_yields_one_row_pinned_to_its_own_fact() {
    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    expressed(&mut facts, "want-1", "把周报写了", 3);
    expressed(&mut facts, "want-2", "看一遍日志", 7);

    let rows = rows(&facts);
    assert_eq!(rows.len(), 2);
    let first = &rows[0];
    assert_eq!(first.name, PROMOTE_DEBT);
    assert_eq!(first.port, PROMOTE_PORT);
    assert_eq!(first.id, "want-1");
    assert_eq!(
        first.when_id.as_ref(),
        Some(&(EVENT_CONATION_EXPRESSED.to_string(), "want-1".to_string())),
        "债只在这一条欲还活着时重算"
    );
    assert_eq!(first.payload["intent_id"], "want-1");
    assert_eq!(first.payload["goal"], "把周报写了");
    assert_eq!(first.payload["source_seq"], 3);
}

#[test]
fn an_expression_without_a_trigger_anchor_is_not_a_debt() {
    let mut facts = DebtFacts::default();
    config(&mut facts, "cfg-1", json!(true));
    facts.note_fact(EVENT_CONATION_EXPRESSED, "want-orphan");
    facts.note_fact_payload(
        EVENT_CONATION_EXPRESSED,
        "want-orphan",
        json!({ "goal": "来路不明" }),
    );
    assert!(rows(&facts).is_empty(), "没有 source_seq 升不了格");
}
