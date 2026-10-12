//! `span` 投影验收——会话级跨度口径（首末事件 ts）、as-of、跨会话中位数合并。
//!
//! 反向用例（改输入会变红）：as-of 之后的 events 不计入；只有 0/1 事件的会话
//! 不进中位数（「没有任务」不得读成「零时长任务」）；无样本 ⇒ `None`（有据的空）。

use super::span;
use crate::symbio_core::projection::span::{SpanSummary, SpanView};
use crate::symbio_core::{Budget, Entity, Event, EventStore, Seq, Store, Verb, EVENT_USER_MESSAGE};

fn msg(id: &str, ts: i64) -> Event {
    Event::pending(id, EVENT_USER_MESSAGE, Entity::Turn, Verb::Opened, 0, "user")
        .with_ts(ts)
        .with_payload(serde_json::json!({ "text": "问" }))
}

/// 口径：跨度 = 末事件 ts − 首事件 ts；事件条数随 as-of 之后的切片计。
#[test]
fn span_is_last_minus_first_ts() {
    let store = EventStore::new();
    store.append(msg("e0", 1_000)).unwrap();
    store.append(msg("e1", 1_500)).unwrap();
    store.append(msg("e2", 4_200)).unwrap();
    let v = span()
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value;
    assert_eq!(v.events, 3);
    assert_eq!(v.span_ms, 4_200 - 1_000);
}

/// as-of：`ts > now` 的事件一概不见（与其余投影同口径）——首末与条数都只算看得见的。
#[test]
fn as_of_hides_future_events() {
    let store = EventStore::new();
    store.append(msg("e0", 1_000)).unwrap();
    store.append(msg("e1", 2_000)).unwrap();
    store.append(msg("e2", 9_000)).unwrap();
    let v = span().apply(&store.range(Seq::new(0)), 2_500, Budget::generous()).value;
    assert_eq!(v.events, 2, "as-of = 2500 ⇒ 只见前两条");
    assert_eq!(v.span_ms, 1_000, "跨度也只按可见的首末算");
}

/// 空事实源 ⇒ 有据的零（0 事件、0 跨度），不是错误。
#[test]
fn empty_source_is_a_honest_zero() {
    let store = EventStore::new();
    let v = span()
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value;
    assert_eq!(v, SpanView::default());
}

/// 单事件 ⇒ 跨度为 0 且**不进中位数分母**（「还没有任务」不是「零时长任务」）。
#[test]
fn single_event_sessions_do_not_drag_the_median() {
    let one = SpanView {
        events: 1,
        span_ms: 0,
    };
    let two = SpanView {
        events: 2,
        span_ms: 5_000,
    };
    let s = SpanView::merge(&[one, two]);
    assert_eq!(s.sessions, 1, "0/1 事件的会话不进分母");
    assert_eq!(s.spans, vec![5_000]);
    assert_eq!(s.median_ms(), Some(5_000));
}

/// 中位数 = P50 最近邻取法（与 `TierLatency::percentile` 同一）：偶数样本取下中位。
#[test]
fn median_uses_nearest_neighbor() {
    let s = SpanSummary {
        sessions: 4,
        spans: vec![10, 20, 30, 40],
    };
    assert_eq!(s.median_ms(), Some(20), "4 样本的 P50 = 第 2 个（ceil(2)-1 = 1）");
    let s3 = SpanSummary {
        sessions: 3,
        spans: vec![10, 20, 30],
    };
    assert_eq!(s3.median_ms(), Some(20), "3 样本的 P50 = 正中");
}

/// 无样本 ⇒ `None`——「实例里没有会话」与「会话时长 0ms」是两件事。
#[test]
fn no_samples_is_none_not_zero() {
    let s = SpanSummary::default();
    assert_eq!(s.median_ms(), None);
}

/// 合并的确定性（N1）：同输入逐字节同输出（输入顺序无关——排序在合并内）。
#[test]
fn merge_is_deterministic() {
    let a = SpanView {
        events: 3,
        span_ms: 700,
    };
    let b = SpanView {
        events: 2,
        span_ms: 100,
    };
    let c = SpanView {
        events: 4,
        span_ms: 300,
    };
    let s1 = SpanView::merge(&[a.clone(), b.clone(), c.clone()]);
    let s2 = SpanView::merge(&[c, b, a]);
    assert_eq!(s1, s2);
    assert_eq!(s1.spans, vec![100, 300, 700], "升序是分位数取法的前提");
}
