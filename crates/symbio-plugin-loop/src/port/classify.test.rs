//! 判决往返的入账形状（验收口径 2）：两条用例各钉一侧。
//!
//! 正向那条钉「打了模型 ⇒ 三格齐且带 `via`」，反向那条钉「规则短路 ⇒ 零格」。
//! 两条都跑**同一个**派发点（[`classify_round_trip`]）——反向用例不是「按构造
//! 不可达」：它真的调了那个函数，断言的是它**没有**落任何一格。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use symbio_core::{EventStore, Seq, Store};
use symbio_kernel::dialog::{DecideOutcome, DecisionPlan, Verdict};

use super::super::Round;
use super::classify_round_trip;

/// 一笔判决往返的账目位置（三格与端口 `model` 的 id 同一口径）。
fn round() -> Round<'static> {
    Round {
        turn: 3,
        produced_by: 2,
        drive_seq: 9,
        now: 0,
        step: "c0",
        via: "classify/decide",
    }
}

/// 一份判决往返的计划（提示词 + 消息 + 模型）。
fn plan() -> DecisionPlan {
    DecisionPlan {
        system_prompt: "分类".into(),
        messages: vec![],
        model_id: None,
    }
}

#[tokio::test]
async fn the_judgment_round_trip_is_accounted_as_three_cells_with_its_via() {
    // 判据 2 的正向：判决往返入账 ⇒ 三格（`request` → `sent` → `response`），
    // 且三格都带 `via = "classify/decide"`——判据侧靠它把内部往返与「这一轮自己
    // 的模型往返」分开（混进来会让一条判决往返被读成「这一轮答完了」）。
    let store = EventStore::new();
    let decision = classify_round_trip(
        &store,
        round(),
        DecideOutcome::NeedsModel { plan: plan() },
        |_plan| async { Ok("work".to_string()) },
        |text| {
            let text = text.to_string();
            async move {
                Verdict::Escalate {
                    reason: if text == "work" {
                        "needs_work".into()
                    } else {
                        "unclassified".into()
                    },
                }
            }
        },
    )
    .await;

    assert_eq!(decision.model_calls, 1, "打了一次模型，次数就是 1");
    assert_eq!(
        decision.verdict,
        Verdict::Escalate {
            reason: "needs_work".into()
        }
    );
    let facts = store.range(Seq::new(0));
    assert_eq!(
        facts.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec!["request", "sent", "response"]
    );
    for e in &facts {
        assert_eq!(
            e.payload["via"],
            json!("classify/decide"),
            "{} 这一格丢了身份",
            e.kind
        );
    }
    // 应答格的 `calls` 是**空数组**（内部往返不带工具）：不是缺席。
    assert_eq!(facts[2].payload["calls"], json!([]));
}

#[tokio::test]
async fn the_rule_shortcut_leaves_no_accounting_cell_at_all() {
    // 判据 2 的反向（不许用「按构造不可达」代替）：规则短路 ⇒ **零入账格**。
    // 这是 D-122 决策 4 的形状——「行不成立」，不是「行成立但没写格」。
    let store = EventStore::new();
    // 旗子把「模型没被打」变成可断言的事实：panic 只能证明「没走到那一步」。
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    let decision = classify_round_trip(
        &store,
        round(),
        DecideOutcome::Decided {
            verdict: Verdict::Answered {
                reason: "greeting".into(),
            },
        },
        move |_plan| {
            let flag = flag.clone();
            async move {
                flag.store(true, Ordering::SeqCst);
                Ok("work".to_string())
            }
        },
        |_| async { unreachable!("规则短路没有应答可解析") },
    )
    .await;

    assert!(!called.load(Ordering::SeqCst), "规则短路不许打模型");
    assert_eq!(decision.model_calls, 0);
    assert_eq!(
        decision.verdict,
        Verdict::Answered {
            reason: "greeting".into()
        }
    );
    let facts = store.range(Seq::new(0));
    let kinds: Vec<&str> = facts.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.is_empty(), "规则短路一条格都不该落：{kinds:?}");
}
