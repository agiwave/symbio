//! `store` 单测 —— S0 出口判据 C1（`seq` 单调断言，[plan/04 §4](../../../../docs/plan/04-工程落地.md)）。
//!
//! 判定纪律：每条断言配**反向用例**——把检查关掉（或喂坏输入），违规必须可见；
//! 检测不到 = 这条检查是摆设。

use super::*;
use crate::symbio_core::event::{Entity, Event, EventEnvelope, Verb};

fn ev(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, entity, verb, turn, "main")
}

// ── C1：seq 严格单调、无跳号（I1 的存储侧） ─────────────────────────────

#[test]
fn seq_is_strictly_monotonic_without_gaps() {
    let store = EventStore::new();
    let mut seqs = Vec::new();
    for i in 0..10 {
        let e = ev(
            &format!("e{i}"),
            "user.message",
            Entity::Turn,
            Verb::Opened,
            i as u64,
        );
        let s = store.append(e).expect("首插必成");
        seqs.push(s.value());
    }
    // 严格递增且无跳号：第 k 条的 seq 恰为 k。
    for (k, v) in seqs.iter().enumerate() {
        assert_eq!(*v, k as u64, "第 {k} 次写入的 seq 应恰为 {k}（无跳号）");
    }
}

#[test]
fn head_equals_event_count_and_range_reads_prefix_window() {
    let store = EventStore::new();
    assert_eq!(store.head().value(), 0, "空库 head = 0（下一个待分配）");
    for i in 0..5 {
        let e = ev(
            &format!("e{i}"),
            "user.message",
            Entity::Turn,
            Verb::Opened,
            i as u64,
        );
        store.append(e).expect("首插必成");
    }
    assert_eq!(store.head().value(), 5);
    assert_eq!(store.len(), 5);

    // range(from) = [from, head)：from=2 ⇒ 恰为 seq 2..5 的三条。
    let window = store.range(Seq::new(2));
    assert_eq!(window.len(), 3);
    assert_eq!(window[0].seq().expect("已入库必有 seq").value(), 2);
    assert_eq!(window[2].seq().expect("已入库必有 seq").value(), 4);

    // from 超界 ⇒ 空区间，而不是 panic。
    assert!(store.range(Seq::new(999)).is_empty());
    // from=0 ⇒ 全量。
    assert_eq!(store.range(Seq::new(0)).len(), 5);
}

// ── 幂等：event_id 重复 ⇒ Duplicate，且不消耗 seq（不产生跳号） ─────────

#[test]
fn duplicate_event_id_returns_duplicate_and_consumes_no_seq() {
    let store = EventStore::new();
    let e1 = ev("same-id", "user.message", Entity::Turn, Verb::Opened, 1);
    let dup = ev("same-id", "user.message", Entity::Turn, Verb::Opened, 1);
    store.append(e1).expect("首插必成");
    assert_eq!(store.append(dup), Err(AppendError::Duplicate));
    assert_eq!(
        store.head().value(),
        1,
        "重复插入不得推进 head（否则产生跳号）"
    );

    // 反向用例：换一个 id 就能插入，且 seq 接续为 1（无跳号的直接证据）。
    let e2 = ev(
        "other-id",
        "turn.state_changed",
        Entity::Turn,
        Verb::Progressed,
        1,
    );
    let s = store.append(e2).expect("换 id 必成");
    assert_eq!(s.value(), 1);
}

// ── append-only 的构造侧：入库事件的 seq 不可被外部改写 ─────────────────

#[test]
fn caller_cannot_forge_or_reassign_seq() {
    let store = EventStore::new();
    let e = ev("e0", "user.message", Entity::Turn, Verb::Opened, 0);
    let s = store.append(e).expect("首插必成");
    let stored = &store.range(Seq::new(0))[0];
    assert_eq!(stored.seq(), Some(s));
    // 调用方拿到的克隆体改不了库里的 seq——range 返回快照。
    let mut forged = stored.clone();
    forged.assign_seq(Seq::new(999));
    assert_eq!(
        store.range(Seq::new(0))[0].seq(),
        Some(s),
        "库内 seq 不受外部影响"
    );
}

// ── 信封契约：入库前 seq = None ────────────────────────────────────────

#[test]
fn pending_event_has_no_seq_until_appended() {
    let e = ev("e0", "user.message", Entity::Turn, Verb::Opened, 0);
    assert_eq!(e.seq(), None, "未入库事件不得自带 seq（由 Store 分配）");
}
