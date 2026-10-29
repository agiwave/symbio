//! `slo_report` 验收——时延列口径（final 实测、按 turn 归档、兜底不混入）；
//! 末尾附**阶段三扫描口**（opt-in：walk 各会话的 v2 WAL，三投影同源出报告）。

use super::slo_report;
use super::{SloLatencyView, TierLatency};
use crate::declarations::CHAT_VOCAB as CHAT;
use crate::projections::fallback::{fallback_rate, FallbackRateView, TierStats};
use symbio_core::{cost_ledger, Budget, Entity, Event, EventStore, Seq, Store, Verb};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

fn user_msg(id: &str, turn: u64, tier: &str) -> Event {
    Event::pending(
        id,
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "问", "tier": tier }))
}

fn final_evt(id: &str, turn: u64, produced: u64, cost_ms: u64) -> Event {
    Event::pending(
        id,
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "session/main",
    )
    .with_produced_by(produced)
    .with_cost_ms(cost_ms)
}

/// 分位数取法：最近邻（`ceil(p/100 × n) - 1`）——P50 = 中位、P100 = max。
#[test]
fn percentile_uses_nearest_neighbor() {
    let t = TierLatency {
        tier: "deep".into(),
        samples: vec![10, 20, 30, 40],
    };
    assert_eq!(t.p50(), 20, "4 样本的 P50 = 第 2 个（ceil(2)-1 = 1）");
    assert_eq!(t.p95(), 40, "P95 落在最大样本");
    assert_eq!(t.percentile(100), 40, "P100 = max");
    assert_eq!(t.percentile(0), 0, "P0 恒 0");
    assert_eq!(TierLatency::default().p50(), 0, "空档位不报警");
}

/// 时延口径：final 的 cost_ms 按本轮声明的档位归桶；**兜底轮不进样本**；
/// 无档位声明进 `unspecified`（可观测）。
#[test]
fn latency_samples_group_by_declared_tier_and_exclude_fallback() {
    let store = EventStore::new();
    // turn 0：deep，两轮成功（2000 / 4000ms）。
    store.append(user_msg("u0", 0, "deep")).unwrap();
    let s0 = store.head().value();
    store.append(final_evt("f0", 0, s0, 2000)).unwrap();
    store.append(user_msg("u1", 1, "deep")).unwrap();
    let s1 = store.head().value();
    store.append(final_evt("f1", 1, s1, 4000)).unwrap();
    // turn 2：无 tier 声明 ⇒ unspecified。
    store
        .append(
            Event::pending(
                "u2",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                2,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "问" })),
        )
        .unwrap();
    let s2 = store.head().value();
    store.append(final_evt("f2", 2, s2, 8000)).unwrap();
    // turn 3：deep 走兜底——cost_ms 3000 **不得**进时延样本。
    store.append(user_msg("u3", 3, "deep")).unwrap();
    let s3 = store.head().value();
    store
        .append(
            Event::pending(
                "fb3",
                EVENT_CHAT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                3,
                "session/main",
            )
            .with_produced_by(s3)
            .with_cost_ms(3000)
            .with_payload(serde_json::json!({ "reason": "fallback", "detail": "上游 500" })),
        )
        .unwrap();

    let snapshot = store.range(Seq::new(0));
    let view = slo_report(CHAT)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    let deep = view.of("deep");
    assert_eq!(deep.samples, vec![2000, 4000], "deep 只收成功轮");
    assert_eq!((deep.p50(), deep.p99()), (2000, 4000));

    let unspec = view.of("unspecified");
    assert_eq!(
        unspec.samples,
        vec![8000],
        "漏声明的档位进 unspecified（可观测）"
    );

    assert_eq!(view.of("reflect").count(), 0, "空档位 = 空账不是错误");

    // N1：双跑逐字节一致。
    let again = slo_report(CHAT)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    assert_eq!(view, again);
}

/// 三列同源（阶段三报告的形态）：时延 + 兜底率 + 成本台账出自同一事件切片。
#[test]
fn three_slo_columns_share_one_event_slice() {
    let store = EventStore::new();
    store.append(user_msg("u0", 0, "deep")).unwrap();
    let s0 = store.head().value();
    store.append(final_evt("f0", 0, s0, 5000)).unwrap();

    let snapshot = store.range(Seq::new(0));
    let latency = slo_report(CHAT)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let fr = fallback_rate(CHAT)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let ledger = cost_ledger()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    assert_eq!(latency.of("deep").samples, vec![5000]);
    assert_eq!((fr.of("deep").turns, fr.of("deep").fallbacks), (1, 0));
    assert_eq!(ledger.of("session/main").spent_ms, 5000);
}

// ── 跨会话合并（P2：聚合逻辑升为生产代码，[plan/13 批 P2]）──────────────────
//
// ## 为什么单测在这里而不是只在 e2e
//
// 合并是**纯函数**，而它承载两条「错了不会报错」的决定：① 样本是并集不是覆盖；
// ② 计是相加不是取新。两条反过来都静默——并集错成覆盖会让 P99 只反映最后一个
// 会话，计数错成取新会让兜底率随扫描顺序变。e2e 只验「两个会话都读到了」，
// 这两种错它一个都照不出来，所以必须在纯函数层穷举。

/// 两份视图的样本**并集**（同档位），不是覆盖。
#[test]
fn merge_unions_samples_per_tier_rather_than_overwriting() {
    let mk = |tier: &str, xs: &[u64]| SloLatencyView {
        by_tier: [(
            tier.to_string(),
            TierLatency {
                tier: tier.into(),
                samples: xs.to_vec(),
            },
        )]
        .into_iter()
        .collect(),
    };
    let a = mk("deep", &[10, 30]);
    let b = mk("deep", &[20]);
    let c = mk("fast", &[5]);

    let merged = SloLatencyView::merge(&[a, b, c]);
    assert_eq!(
        merged.of("deep").samples,
        vec![10, 20, 30],
        "并集且升序（percentile 的前提）"
    );
    assert_eq!(
        merged.of("fast").samples,
        vec![5],
        "只在一个输入里的档位照常进结果"
    );
    // 合并后再算分位数 = 全量口径（3 个样本的 P99 是 30，不是任一份自己的）
    assert_eq!(merged.of("deep").p99(), 30);
    assert_eq!(merged.of("deep").count(), 3);
}

/// 档位集合取**并集**，输入里没有的档位不造空账。
#[test]
fn merge_does_not_invent_empty_tiers() {
    let mk = |tier: &str, xs: &[u64]| SloLatencyView {
        by_tier: [(
            tier.to_string(),
            TierLatency {
                tier: tier.into(),
                samples: xs.to_vec(),
            },
        )]
        .into_iter()
        .collect(),
    };
    let merged = SloLatencyView::merge(&[mk("deep", &[1])]);
    assert!(
        !merged.by_tier.contains_key("fast"),
        "没出现过的档位不在结果里（`of()` 负责给空账）"
    );
    // 空输入 ⇒ 有据的空，不是错误
    assert!(SloLatencyView::merge(&[]).by_tier.is_empty());
}

/// 计数**相加**（分子分母都是加法），率是纯查询故合并后再算与分开算逐字相同。
#[test]
fn fallback_merge_adds_counts_and_keeps_the_rate_pure() {
    let mk = |turns: u64, fallbacks: u64| FallbackRateView {
        by_tier: [(
            "deep".to_string(),
            TierStats {
                tier: "deep".into(),
                turns,
                fallbacks,
            },
        )]
        .into_iter()
        .collect(),
    };
    let merged = FallbackRateView::merge(&[mk(10, 1), mk(20, 2)]);
    let st = merged.of("deep");
    assert_eq!(
        (st.turns, st.fallbacks),
        (30, 3),
        "分子分母都相加，不取较新的一份"
    );
    assert_eq!(st.rate(), 0.1, "率在合并后由纯查询算出");
    assert!(
        FallbackRateView::merge(&[]).by_tier.is_empty(),
        "空输入 ⇒ 有据的空"
    );
}

// ── 阶段三扫描口（已退役，2026-10-09，P2）──────────────────────────────────
//
// 原形态：`#[ignore]` + 需 `SYMBIO_SLO_SCAN_ROOTS` 手工跑。**退役理由**：它的
// 全部职责已被两件事接走，且都从「手工 opt-in」变成「每轮门禁自动跑」：
//
//   ① 跨会话**聚合口径**（并集 / 重排 / 相加）⇒ 升为 core 的
//      `SloLatencyView::merge` / `FallbackRateView::merge`，由
//      `merge_unions_samples_per_tier_rather_than_overwriting` 等三条纯函数判据钉；
//   ② **扫描 + 落成读数** ⇒ 生产路径 `crates/symbio-plugin-session/src/stats.rs` 的聚合列
//      （`fleet`），由 `fleet_aggregates_every_session_in_the_instance` 等四条判据钉，
//      其中「只走一层」「坏 WAL 不拖垮整列」是原 `#[ignore]` 测试**从未覆盖**的两条。
//
// 保留它只会多一份要走环境变量的手工入口，而那份入口与生产读数之间没有任何
// 对账——正是「一个事实两个 owner」。故删，不保留 deprecated 形态。
