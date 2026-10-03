//! `invariants` 单测 —— 每条检查都要证明「它红得起来」。
//!
//! 反向用例与本体的关系：正向用例证明检查**在违规时报警**，反向用例证明
//! 合规序列**不报警**（检查不是永远红的状态机）。两组都绿，检查才存在。

use super::*;
use crate::symbio_core::event::{Entity, Event, EventEnvelope, Verb};
use crate::symbio_core::store::{EventStore, Store};

/// 造一条 pending 事件（未入库，无 seq）。
fn pending(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, entity, verb, turn, "main").with_produced_by(0)
}

/// 造一批**已入库**事件（seq 由 Store 分配 ⇒ 天然单调）。
fn stored(events: Vec<Event>) -> Vec<Event> {
    let store = EventStore::new();
    for e in events {
        store.append(e).expect("首插必成");
    }
    store.range(crate::symbio_core::event::Seq::new(0))
}

// ── C1：seq 单调 ───────────────────────────────────────────────────────

#[test]
fn seq_monotonic_passes_on_store_output() {
    let events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("e2", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    assert!(
        seq_monotonic(&events).is_empty(),
        "Store 产出的序列天然单调：{:?}",
        seq_monotonic(&events)
    );
}

#[test]
fn seq_gap_is_detected() {
    let mut events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    // 模拟「中间丢了一条」：删除 seq=1 后，后一条应接 seq=1 却是 seq=2。
    events.remove(1);
    // 重新造：seq 0 与 seq 2 相邻（跳号）。
    let mut e2 = pending("e2", "user.message", Entity::Turn, Verb::Opened, 2);
    e2.assign_seq(crate::symbio_core::event::Seq::new(2));
    events.push(e2);
    let bad = seq_monotonic(&events);
    assert_eq!(bad.len(), 1, "跳号必须被看见");
    assert_eq!(bad[0].event_id, "e2");
    assert!(bad[0].why.contains("严格单调"), "{}", bad[0].why);
}

#[test]
fn pending_events_are_skipped_by_seq_check() {
    // pending（未入库）事件没有 seq，不参与单调性检查——它们还不是事实。
    let events = vec![pending("p0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(seq_monotonic(&events).is_empty());
}

// ── C2 / N3：每 turn 至多 1 条 final ───────────────────────────────────

#[test]
fn one_final_per_turn_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    assert!(final_unique_per_turn(&events).is_empty());
}

#[test]
fn two_finals_in_one_turn_is_detected() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        // 同一 turn 的第二条 final——转写收敛性被破坏。
        pending("f0b", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
    ]);
    let bad = final_unique_per_turn(&events);
    assert_eq!(bad.len(), 1, "一轮两条 final 必须被看见");
    assert_eq!(bad[0].event_id, "f0b");
    assert!(
        bad[0].why.contains("f0"),
        "违规要指出已存在的那条：{}",
        bad[0].why
    );
}

#[test]
fn closed_tasks_do_not_count_as_final() {
    // `task.completed` 也是 closed，但不是发言——N3 只管 turn 的发言收敛。
    let events = stored(vec![
        pending("t0", "task.completed", Entity::Task, Verb::Closed, 0),
        pending("t1", "task.completed", Entity::Task, Verb::Closed, 0),
    ]);
    assert!(final_unique_per_turn(&events).is_empty());
}

// ── C3 / N5：断言类事件必须带溯源 ──────────────────────────────────────
#[test]
fn asserted_with_provenance_passes() {
    let events = stored(vec![pending(
        "v0",
        "classify.verdict",
        Entity::Verdict,
        Verb::Asserted,
        0,
    )
    .with_produced_by(0)]);
    assert!(produced_by_coverage(&events).is_empty());
}

#[test]
fn asserted_without_provenance_is_detected() {
    let no_source = pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0);
    // 显式去掉溯源（pending 默认带 0，这里造一条真正的裸断言）。
    let no_source = Event {
        produced_by: None,
        ..no_source
    };
    let events = stored(vec![no_source]);
    let bad = produced_by_coverage(&events);
    assert_eq!(bad.len(), 1, "裸断言必须被看见（I2 无溯源不声明）");
    assert_eq!(bad[0].event_id, "v0");
}

#[test]
fn non_asserted_events_need_no_provenance() {
    // 普通过程事件（opened 等）不是声明，不要求 produced_by——
    // S1 起收束类（chat.assistant.final/fallback）例外，见下。
    let events = vec![pending("u0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(produced_by_coverage(&events).is_empty());
}

#[test]
fn reply_without_provenance_is_detected() {
    // S01 §5：回复必须有 produced_by——用户收到的话必须可追溯。
    let bare = Event {
        produced_by: None,
        ..pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0)
    };
    let bad = produced_by_coverage(std::slice::from_ref(&bare));
    assert_eq!(bad.len(), 1, "无溯源的答复必须被看见（I2）");
    assert!(bad[0].why.contains("收束"), "{}", bad[0].why);
}

#[test]
fn fallback_without_provenance_is_detected() {
    // 兜底也是一条普通事件——同样逃不出 I2（roadmap/S01 §2 的关键设计）。
    let bare = Event {
        produced_by: None,
        ..pending(
            "fb0",
            "chat.assistant.fallback",
            Entity::Turn,
            Verb::Closed,
            0,
        )
    };
    assert_eq!(produced_by_coverage(std::slice::from_ref(&bare)).len(), 1);
}

#[test]
fn reply_with_provenance_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0).with_produced_by(0),
    ]);
    assert!(produced_by_coverage(&events).is_empty());
}

// ── check_all：三条合跑 ────────────────────────────────────────────────

#[test]
fn check_all_on_clean_sequence_is_empty() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0).with_produced_by(0),
    ]);
    assert!(check_all(&events).is_empty(), "合规序列三查全绿");
}

// ── 故障注入用例集（S1 步骤 4：超时 / 乱序 / 重复 final / 无溯源，4 类违规全被捕获）────
//
// [plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 4 步的产物：每类故障注入
// 事件序列，断言**对应的检查必须红**。反向用例（合规序列不报警）在上方各检查区。

#[test]
fn fault_timeout_turn_never_settles_is_caught() {
    // 超时 = 生成失败且什么都没写——turn 开了却没有 final/fallback。
    let events = stored(vec![pending(
        "u0",
        "user.message",
        Entity::Turn,
        Verb::Opened,
        0,
    )]);
    // 严判档（`false`）：故障注入要的是"开了没关就是红"。
    let bad = unresolved_turns(&events, false);
    assert_eq!(bad.len(), 1, "静默中断必须被看见（I3：兜底必须产生事件）");
    assert!(bad[0].why.contains("未收束"), "{}", bad[0].why);
    // 宽限档（`true`）：切片尾部这一轮可能只是**还在生成**——纯函数无从分辨，
    // 放行（读侧口径，见 `check_all`）。两条判据并存，不是谁覆盖谁。
    assert!(
        check_all(&events).is_empty(),
        "读侧不该把在途尾轮报成静默中断：{:?}",
        check_all(&events)
    );
}

#[test]
fn fault_timeout_avoided_by_fallback_event() {
    // 反向：超时后**写了** fallback → turn 收束，检查放行（兜底是普通事件）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending(
            "fb0",
            "chat.assistant.fallback",
            Entity::Turn,
            Verb::Closed,
            0,
        ),
    ]);
    assert!(unresolved_turns(&events, false).is_empty());
}

#[test]
fn fault_out_of_order_seq_regression_is_caught() {
    // 乱序 = seq 回退（3 之后又来 1）——绕过唯一写入口的典型痕迹。
    let mut e3 = pending("e3", "task.progress", Entity::Task, Verb::Progressed, 0);
    e3 = Event {
        seq: Some(crate::symbio_core::event::Seq::new(3)),
        ..e3
    };
    let mut e1 = pending("e1", "task.progress", Entity::Task, Verb::Progressed, 1);
    e1 = Event {
        seq: Some(crate::symbio_core::event::Seq::new(1)),
        ..e1
    };
    let bad = seq_monotonic(&[e3, e1]);
    // 两条都被看见：seq 3 处应为 0（开头顶格），seq 1 处应为 4（回退）。
    assert_eq!(bad.len(), 2, "乱序注入的两处异常都必须被看见：{:?}", bad);
    assert_eq!(bad[1].event_id, "e1");
}

#[test]
fn fault_double_final_is_caught_by_check_all() {
    // 重复 final 经 check_all 合跑也必须红（CI 形态下无漏网）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
    ]);
    let violations = check_all(&events);
    assert!(
        violations.iter().any(|v| v.why.contains("final")),
        "{:?}",
        violations
    );
}

#[test]
fn fault_missing_provenance_is_caught_by_check_all() {
    // 无溯源的断言经 check_all 合跑也必须红。
    let bare = Event {
        produced_by: None,
        ..pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0)
    };
    let events = stored(vec![bare]);
    assert!(
        check_all(&events).iter().any(|v| v.why.contains("溯源")),
        "{:?}",
        check_all(&events)
    );
}

#[test]
fn fault_budget_exceeded_is_caught() {
    // I3 记账：单事件耗时超主体预算必须被看见（超预算允许发生，但不容隐身）。
    let events =
        vec![pending("s0", "task.progress", Entity::Task, Verb::Progressed, 0).with_cost_ms(2_000)];
    let bad = budget_exceeded(&events, Some(1_500));
    assert_eq!(bad.len(), 1, "超预算必须被看见");
    assert!(bad[0].why.contains("2"), "{}", bad[0].why);
    // 反向：预算内不报警。
    let ok =
        vec![pending("s1", "task.progress", Entity::Task, Verb::Progressed, 0).with_cost_ms(1_499)];
    assert!(budget_exceeded(&ok, Some(1_500)).is_empty());
    // 宽限档：没给预算就无从判"超"——返回空，不是拿默认数硬比。
    assert!(budget_exceeded(&events, None).is_empty(), "无预算 ⇒ 不判");
}

// ── 读侧宽限（04 §3.1 批④：两条 I3 检查接进读出口前先立的口径）────────────

#[test]
fn read_side_graces_the_trailing_open_turn_but_flags_the_overtaken_one() {
    // 轮 0 开了没收束，轮 1 随后开轮并正常收束——会话已经往前走，轮 0 是确凿的
    // 静默中断（不是"还在生成"），读侧必须报。
    let overtaken = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    let bad = unresolved_turns(&overtaken, true);
    assert_eq!(bad.len(), 1, "被后续轮越过的未收束轮必须报");
    assert_eq!(bad[0].event_id, "u0", "违规锚在缺口本身（开轮那条）");
    assert!(
        check_all(&overtaken)
            .iter()
            .any(|v| v.why.contains("未收束")),
        "经 check_all 也要红：{:?}",
        check_all(&overtaken)
    );
}

#[test]
fn read_side_budget_follows_declared_tiers_and_skips_when_none_declared() {
    // 声明了 reflex 档（80ms）⇒ 预算有值，2000ms 的事件必须被看见。
    let declared = stored(vec![
        Event::pending("u0", "user.message", Entity::Turn, Verb::Opened, 0, "user")
            .with_payload(serde_json::json!({ "text": "问", "tier": "reflex" })),
        Event::pending(
            "s0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "agent:autonomous",
        )
        .with_produced_by(0)
        .with_cost_ms(2_000),
    ]);
    assert_eq!(
        declared_budget_ms(&declared),
        Some(80),
        "声明档位 ⇒ 预算 = 该档四层预算表"
    );
    assert!(
        check_all(&declared).iter().any(|v| v.why.contains("超")),
        "读侧必须看见超预算：{:?}",
        check_all(&declared)
    );

    // 一档都没声明 ⇒ `None` ⇒ 不判（没有预算就没有"超"）。
    let undeclared = stored(vec![pending(
        "s1",
        "task.progress",
        Entity::Task,
        Verb::Progressed,
        0,
    )
    .with_cost_ms(9_999_999)]);
    assert_eq!(declared_budget_ms(&undeclared), None);
    assert!(
        check_all(&undeclared).is_empty(),
        "未声明档位的切片首日不该红：{:?}",
        check_all(&undeclared)
    );
}
