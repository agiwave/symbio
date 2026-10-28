//! 兜底率投影的验收（原 core `actors` 用例，随投影搬到 session，DE-025 D-221）。

use super::fallback_rate;
use crate::declarations::CHAT_VOCAB as CHAT;
use symbio_core::{Budget, Entity, Event, EventStore, Seq, Store, Verb};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

/// 深度档 3 turn（2 final + 1 fallback）→ 33%；反射档 2 turn 全 final → 0%；
/// 缺 tier 的 turn 进 unspecified 桶（可观测，不静默归类）；N1 双跑一致。
#[test]
fn fallback_rate_counts_per_tier_and_is_deterministic() {
    let store = EventStore::new();
    // 深度档：t0 final、t1 fallback、t2 final。
    for (i, turn) in [0u64, 1, 2].iter().enumerate() {
        store
            .append(
                Event::pending(
                    format!("u-deep-{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    *turn,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap();
        let kind = if *turn == 1 {
            EVENT_CHAT_ASSISTANT_FALLBACK
        } else {
            EVENT_CHAT_ASSISTANT_FINAL
        };
        store
            .append(
                Event::pending(
                    format!("a-deep-{i}"),
                    kind,
                    Entity::Turn,
                    Verb::Closed,
                    *turn,
                    "session/main",
                )
                .with_produced_by(i as u64 * 2),
            )
            .unwrap();
    }
    // 反射档：t3、t4 全 final。
    for i in 0..2u64 {
        let turn = 3 + i;
        store
            .append(
                Event::pending(
                    format!("u-reflex-{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    turn,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "reflex" })),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    format!("a-reflex-{i}"),
                    EVENT_CHAT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    turn,
                    "session/main",
                )
                .with_produced_by(turn * 2),
            )
            .unwrap();
    }
    // 未声明档位：t5 fallback —— 必须落在 unspecified 桶。
    store
        .append(
            Event::pending(
                "u-x",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                5,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "问" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "a-x",
                EVENT_CHAT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                5,
                "session/main",
            )
            .with_produced_by(10),
        )
        .unwrap();

    let snapshot = store.range(Seq::new(0));
    let view = fallback_rate(CHAT).apply(&snapshot, i64::MAX, Budget::generous());

    let deep = view.value.of("deep");
    assert_eq!((deep.turns, deep.fallbacks), (3, 1));
    assert!((deep.rate() - 1.0 / 3.0).abs() < 1e-9);

    let reflex = view.value.of("reflex");
    assert_eq!((reflex.turns, reflex.fallbacks), (2, 0));
    assert_eq!(reflex.rate(), 0.0);

    let unspec = view.value.of("unspecified");
    assert_eq!((unspec.turns, unspec.fallbacks), (1, 1));
    assert_eq!(unspec.rate(), 1.0, "未声明档位的兜底必须被看见");

    // N1：双跑逐字节一致。
    let again = fallback_rate(CHAT).apply(&snapshot, i64::MAX, Budget::generous());
    assert_eq!(view.value, again.value);
}
