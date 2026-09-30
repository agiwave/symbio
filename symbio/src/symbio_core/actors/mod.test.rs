//! `actors` 单测 —— 含 **S01 闭环彩排**（[plan/04 §3](../../../../docs/plan/04-工程落地.md)
//! 第 3 步：用 `Decider` 跑通完整闭环，零 LLM）。
//!
//! 彩排把 S0 的地基（store / projection / invariants）与 S1 的 ② `actors`
//! 拧成一条完整链路，验收断言逐条对应
//! [roadmap/S01 §6](../../../../docs/plan/roadmap/S01-最小闭环.md)。

use super::*;
use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use crate::symbio_core::invariants::check_all;
use crate::symbio_core::projection::turnstate::turnstate;
use crate::symbio_core::store::{EventStore, Store};
use crate::symbio_core::view::Budget;

// ── ActorSpec / Pattern（01 §4 契约形状） ──────────────────────────────

#[test]
fn trivial_spec_is_the_s01_baseline() {
    let spec = ActorSpec::trivial("agent:main");
    assert_eq!(
        spec.pattern,
        Pattern::Decider,
        "平凡值 = decider（零 LLM 全链路）"
    );
    assert_eq!(spec.budget_ms, 60_000, "平凡值预算：慢但正确");
    assert_eq!(
        spec.scope,
        Scope::Root,
        "平凡值 scope = root（root 是一等输入）"
    );
}

// ── S01 闭环彩排 ────────────────────────────────────────────────────────

/// 一轮完整对话：用户消息入库 → Decider 应答 → final/fallback 入库 → 三查。
///
/// 返回 (store, 收束事件 seq)；`miss` 模拟生成失败（Decider 查空规则表）。
fn rehearse_turn(utterance: &str, decider: &Decider) -> (EventStore, Result<u64, u64>) {
    let store = EventStore::new();
    // 「收到」：用户消息成为一条事件（turn × opened）。
    let seq = store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": utterance })),
        )
        .expect("用户消息必入库");
    // 「思考 + 发出」：actor 只看事件切片（View 的原始形态）。
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let outcome = match decider.respond(&snapshot) {
        Ok(reply) => {
            let s = store
                .append(
                    Event::pending(
                        "f0",
                        EVENT_ASSISTANT_FINAL,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(seq.value())
                    .with_payload(serde_json::json!({ "text": reply })),
                )
                .expect("final 必入库");
            Ok(s.value())
        }
        Err(_) => {
            // 「兜底」：也是一条普通事件，落同一个 turn × closed 格子，同样带溯源。
            let s = store
                .append(
                    Event::pending(
                        "fb0",
                        EVENT_ASSISTANT_FALLBACK,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(seq.value())
                    .with_payload(serde_json::json!({ "text": "抱歉，我暂时答不上来。" })),
                )
                .expect("fallback 必入库");
            Err(s.value())
        }
    };
    (store, outcome)
}

/// 验收 1：一条 `user.message` → 恰好一条 `turn/closed`。
#[test]
fn one_user_message_yields_exactly_one_final() {
    let (store, _) = rehearse_turn("你好，帮我看看", &Decider::rehearsal());
    let finals: Vec<_> = store
        .range(crate::symbio_core::event::Seq::new(0))
        .into_iter()
        .filter(|e| e.kind == EVENT_ASSISTANT_FINAL)
        .collect();
    assert_eq!(finals.len(), 1, "发出端点：发言只有一条 final");
    // 三条不变量在整条链路上全绿（含 I2：final 带溯源）。
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// 验收 2：强制生成失败 → 必须存在 fallback，且它的 `produced_by` 非空。
#[test]
fn forced_failure_produces_fallback_with_provenance() {
    let (store, outcome) = rehearse_turn("这是规则表外的一句话", &Decider::new(vec![]));
    assert!(
        outcome.is_err(),
        "规则未命中 ⇒ 走兜底（Miss 不是错误，是触发条件）"
    );
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let fallbacks: Vec<_> = snapshot
        .iter()
        .filter(|e| e.kind == EVENT_ASSISTANT_FALLBACK)
        .collect();
    assert_eq!(fallbacks.len(), 1, "兜底必须产生事件——静默中断 = 违例");
    assert!(fallbacks[0].produced_by.is_some(), "兜底同样可被 I2 审计");
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// 验收 3：投影两次运行结果逐字节相同。
#[test]
fn turnstate_projection_is_deterministic() {
    let (store, _) = rehearse_turn("你好", &Decider::rehearsal());
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let p = turnstate();
    let v1 = p.apply(&snapshot, 1_000, Budget::generous());
    let v2 = p.apply(&snapshot, 1_000, Budget::generous());
    assert_eq!(v1, v2, "N1：同一事件序列 + 同一参数 ⇒ 逐字节相同");
    assert!(
        v1.value.opened && v1.value.settled(),
        "当前 turn 已开且已收束"
    );
    assert!(v1.value.final_text.is_some(), "规则命中 ⇒ final 态");
}

/// 验收 4（反向）：注入两条 final → N3 必须报警。
#[test]
fn two_finals_are_caught_by_n3() {
    let store = EventStore::new();
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "user",
        ))
        .expect("首插必成");
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    store
        .append(
            Event::pending(
                "f1",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let violations = check_all(&snapshot);
    assert!(
        violations.iter().any(|v| v.why.contains("final")),
        "同一 turn 两条 final 必须被看见：{:?}",
        violations
    );
    // 投影侧的孪生判定也必须不认这个 turn 为「已合法收束」。
    let v = turnstate().apply(&snapshot, 0, Budget::generous());
    assert!(
        !v.value.settled(),
        "settled 的 xor 语义：两条 final ≠ 合法收束"
    );
}

// ── Decider 的输入契约 ─────────────────────────────────────────────────

#[test]
fn decider_reads_the_last_user_message_from_events() {
    // Decider 自己从事件切片里找 user.message——「收到」是它的输入契约。
    let events = vec![Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好" }))];
    assert_eq!(
        Decider::rehearsal().respond(&events).unwrap(),
        "你好，我能做什么？"
    );
}

#[test]
fn decider_miss_reports_the_utterance() {
    let events = vec![Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "规则表外" }))];
    let miss = Decider::rehearsal().respond(&events).unwrap_err();
    assert_eq!(
        miss.utterance, "规则表外",
        "Miss 携带输入摘要（可观测，入兜底载荷）"
    );
}
