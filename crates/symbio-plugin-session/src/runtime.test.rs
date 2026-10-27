//! `runtime::project` 的判别表：哪些格改变结局、哪些不改变、最后一条证据胜出。

use super::*;
use serde_json::json;
use symbio_core::{Entity, Verb};

/// 一条生命周期事件（`turn` 分桶与投影无关，取 1）。
fn ev(kind: &str, verb: Verb, payload: serde_json::Value) -> Event {
    Event::pending("e", kind, Entity::Turn, verb, 1, "test").with_payload(payload)
}

fn opened() -> Event {
    ev(
        EVENT_USER_MESSAGE,
        Verb::Opened,
        json!({ "text": "问", "id": "u-1" }),
    )
}

fn final_with(status: &str) -> Event {
    ev(
        EVENT_ASSISTANT_FINAL,
        Verb::Closed,
        json!({ "status": status, "text": "答", "id": "a-1" }),
    )
}

fn fallback(detail: &str) -> Event {
    ev(
        EVENT_ASSISTANT_FALLBACK,
        Verb::Closed,
        json!({ "reason": "fallback", "detail": detail, "text": "兜底", "id": "fb-1" }),
    )
}

fn progress() -> Event {
    ev(
        EVENT_ASSISTANT_PROGRESS,
        Verb::Progressed,
        json!({ "status": "pending", "id": "p-1" }),
    )
}

/// 根 run 节点的 JSON（`is_run()` + 无父节点即轮次骨架）。
fn run_node(status: &str, error: Option<&str>, parent: Option<&str>) -> serde_json::Value {
    json!({
        "id": "run-1",
        "parent_id": parent,
        "type": "skeleton",
        "status": status,
        "error": error,
        "meta": { "style_tag": "run" },
    })
}

fn edited(node: serde_json::Value) -> Event {
    ev(
        EVENT_MESSAGE_EDITED,
        Verb::Progressed,
        json!({ "message": node }),
    )
}

fn added(nodes: Vec<serde_json::Value>) -> Event {
    ev(
        EVENT_MESSAGE_ADDED,
        Verb::Opened,
        json!({ "messages": nodes }),
    )
}

fn outcome(events: &[Event]) -> Option<RuntimeOutcome> {
    project(events).outcome
}

// ==================== 空 / 开轮 ====================

#[test]
fn empty_history_has_no_outcome() {
    assert_eq!(outcome(&[]), None);
}

#[test]
fn an_open_turn_alone_clears_the_outcome() {
    assert_eq!(outcome(&[opened()]), None);
}

#[test]
fn a_new_user_message_discards_the_previous_round() {
    assert_eq!(
        outcome(&[opened(), final_with("completed"), opened()]),
        None,
        "新一轮开始 ⇒ 上一轮结局作废（与内存写穿缓存的 begin_turn 同口径）"
    );
}

// ==================== 收束格 ====================

#[test]
fn final_completed_settles_completed() {
    assert_eq!(
        outcome(&[opened(), final_with("completed")]),
        Some(RuntimeOutcome::Completed)
    );
}

#[test]
fn final_aborted_settles_aborted() {
    assert_eq!(
        outcome(&[opened(), final_with("aborted")]),
        Some(RuntimeOutcome::Aborted)
    );
}

#[test]
fn final_failed_carries_the_detail() {
    let e = ev(
        EVENT_ASSISTANT_FINAL,
        Verb::Closed,
        json!({ "status": "failed", "reason": "fallback", "detail": "工具崩了" }),
    );
    assert_eq!(
        outcome(&[opened(), e]),
        Some(RuntimeOutcome::Failed {
            error: Some("工具崩了".to_string())
        })
    );
}

#[test]
fn fallback_settles_failed_with_the_why() {
    assert_eq!(
        outcome(&[opened(), fallback("首轮失败")]),
        Some(RuntimeOutcome::Failed {
            error: Some("首轮失败".to_string())
        }),
        "兜底话术递给用户的那一轮在会话层是失败结局"
    );
}

#[test]
fn progress_settles_completed() {
    assert_eq!(
        outcome(&[opened(), progress()]),
        Some(RuntimeOutcome::Completed),
        "等待用户的那一轮以 Ok 收尾，结局是 completed"
    );
}

// ==================== 根 run 节点的收口 ====================

#[test]
fn edited_root_run_failed_is_evidence() {
    assert_eq!(
        outcome(&[
            opened(),
            edited(run_node("failed", Some("持久化失败"), None))
        ]),
        Some(RuntimeOutcome::Failed {
            error: Some("持久化失败".to_string())
        })
    );
}

#[test]
fn edited_root_run_aborted_is_evidence() {
    assert_eq!(
        outcome(&[opened(), edited(run_node("aborted", None, None))]),
        Some(RuntimeOutcome::Aborted)
    );
}

#[test]
fn a_non_root_run_node_is_not_evidence() {
    assert_eq!(
        outcome(&[
            opened(),
            edited(run_node("failed", Some("子轮失败"), Some("turn-1")))
        ]),
        None,
        "只有轮次骨架的定稿才是结局证据（persist_failure 只写根）"
    );
}

#[test]
fn a_completed_run_node_is_not_evidence() {
    assert_eq!(
        outcome(&[opened(), edited(run_node("completed", None, None))]),
        None,
        "正常完成由 final 格承载；run 节点只贡献失败 / 中止两种收口"
    );
}

#[test]
fn a_non_run_skeleton_is_not_evidence() {
    let tool = json!({
        "id": "tool-1",
        "type": "skeleton",
        "status": "failed",
        "error": "工具失败",
        "meta": { "style_tag": "tool" },
    });
    assert_eq!(outcome(&[opened(), edited(tool)]), None);
}

#[test]
fn added_root_run_aborted_is_evidence() {
    assert_eq!(
        outcome(&[opened(), added(vec![run_node("aborted", None, None)])]),
        Some(RuntimeOutcome::Aborted),
        "converge_inflight 以诞生形态补发的骨架同样承载收口"
    );
}

#[test]
fn added_batch_takes_the_last_root_run_node() {
    assert_eq!(
        outcome(&[
            opened(),
            added(vec![
                run_node("aborted", None, None),
                run_node("failed", Some("后写的赢"), None),
            ])
        ]),
        Some(RuntimeOutcome::Failed {
            error: Some("后写的赢".to_string())
        })
    );
}

// ==================== 顺序与忽略 ====================

#[test]
fn the_last_evidence_wins() {
    assert_eq!(
        outcome(&[
            opened(),
            final_with("completed"),
            edited(run_node("failed", Some("终态改写"), None)),
        ]),
        Some(RuntimeOutcome::Failed {
            error: Some("终态改写".to_string())
        })
    );
}

#[test]
fn non_lifecycle_events_do_not_move_the_outcome() {
    let noise = [
        ev(
            symbio_core::EVENT_MESSAGE_REMOVED,
            Verb::Closed,
            json!({ "id": "m-1" }),
        ),
        ev(
            symbio_plugin_api::events::EVENT_TASK_PROGRESS,
            Verb::Progressed,
            json!({ "task_id": "t-1" }),
        ),
        ev(
            symbio_core::EVENT_ASSISTANT_REPORTED,
            Verb::Progressed,
            json!({ "text": "汇报" }),
        ),
    ];
    assert_eq!(
        outcome(&[
            opened(),
            final_with("aborted"),
            noise[0].clone(),
            noise[1].clone(),
            noise[2].clone()
        ]),
        Some(RuntimeOutcome::Aborted)
    );
}
