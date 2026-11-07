//! 终止写方的谓词三态（D-367）：非 `NoDebt` 不落 / 有终答才落 / 同 `root_id` 只落一次。
//! `reason` 原样取终答那一格；终答缺 `reason` 时宁可不落，也不编一个值盖上 I3 缺口。

use symbio_kernel::event::{Entity, Verb};
use symbio_plugin_api::drive::{DriveFact, DriveKey};
use symbio_plugin_api::termination::{TerminationView, TerminationWriter};

use super::SessionTermination;

const ROOT: &str = "u-1";

fn fact(kind: &str, payload: serde_json::Value) -> DriveFact {
    DriveFact {
        kind: kind.to_string(),
        key: DriveKey {
            port: String::new(),
            id: format!("{kind}:{ROOT}"),
            attempt: 1,
        },
        payload,
        at_ms: 0,
        reply: None,
    }
}

fn final_fact() -> DriveFact {
    fact(
        symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
        serde_json::json!({ "root_id": ROOT, "reason": "completed" }),
    )
}

fn terminated_fact() -> DriveFact {
    fact(
        super::EVENT_SESSION_TERMINATED,
        serde_json::json!({ "root_id": ROOT }),
    )
}

fn observe(facts: &[DriveFact], exhausted: bool) -> Vec<symbio_plugin_api::FactDraft> {
    SessionTermination.observe(&TerminationView { facts, exhausted })
}

#[test]
fn non_exhausted_never_terminates() {
    // 驱动前观察恒为 `exhausted = false`：即便终答已在投影里，也不能宣布算完。
    let facts = vec![final_fact()];
    assert!(observe(&facts, false).is_empty());
}

#[test]
fn final_plus_exhausted_lands_one_closing_fact() {
    let facts = vec![final_fact()];
    let drafts = observe(&facts, true);
    assert_eq!(drafts.len(), 1, "终答 ∧ 无债应落且只落一条终止事实");
    let draft = &drafts[0];
    assert_eq!(draft.kind, super::EVENT_SESSION_TERMINATED);
    assert_eq!(
        draft.key,
        format!("{}:{ROOT}", super::EVENT_SESSION_TERMINATED)
    );
    assert_eq!(draft.entity, Entity::Scope);
    assert_eq!(draft.verb, Verb::Closed);
    assert_eq!(draft.payload["root_id"], serde_json::json!(ROOT));
    assert_eq!(
        draft.payload["reason"],
        serde_json::json!("completed"),
        "终止格的 reason 必须原样取终答那一格（不另起一个值）"
    );
}

#[test]
fn final_without_reason_does_not_terminate() {
    // 终答缺 `reason` 是它自己的 I3 缺口：本写方不落格，也不编一个值盖住它。
    let facts = vec![fact(
        symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
        serde_json::json!({ "root_id": ROOT }),
    )];
    assert!(observe(&facts, true).is_empty());
}

#[test]
fn already_terminated_is_idempotent() {
    // 同一事实集重复驱动只得到同一结论（§5 的幂等判据）。
    let facts = vec![final_fact(), terminated_fact()];
    assert!(observe(&facts, true).is_empty());
}

#[test]
fn no_final_does_not_terminate() {
    // 空转收束（无终答）不是「算完」。
    assert!(observe(&[], true).is_empty());
}
