//! `slo_report` 验收——时延列口径（final 实测、按 turn 归档、兜底不混入）；
//! 末尾附**阶段三扫描口**（opt-in：walk 各会话的 v2 WAL，三投影同源出报告）。

use super::super::fallback::fallback_rate;
use super::slo_report;
use crate::symbio_core::{
    cost_ledger, Budget, Entity, Event, EventStore, Seq, Store, TierLatency, Verb,
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
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
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "agent:main",
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
                EVENT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                3,
                "agent:main",
            )
            .with_produced_by(s3)
            .with_cost_ms(3000)
            .with_payload(serde_json::json!({ "why": "上游 500" })),
        )
        .unwrap();

    let snapshot = store.range(Seq::new(0));
    let view = slo_report()
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
    let again = slo_report()
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
    let latency = slo_report()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let fr = fallback_rate()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let ledger = cost_ledger()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    assert_eq!(latency.of("deep").samples, vec![5000]);
    assert_eq!((fr.of("deep").turns, fr.of("deep").fallbacks), (1, 0));
    assert_eq!(ledger.of("agent:main").spent_ms, 5000);
}

// ── 阶段三扫描口（opt-in）────────────────────────────────────────────────────
//
// 用法：设 `SYMBIO_SLO_SCAN_ROOTS`（**分号**分隔的目录列表——Windows 盘符
// 自带冒号，`:` 不能当分隔符；通常是会话存储根，扫描器在其下递归找
// `v2-events.wal`，即 v2 事实桥的落盘处），然后
//   cargo test -p symbio --lib slo_scan_wal_roots -- --ignored --nocapture
// 输出 P50/P95/P99、兜底率与成本台账——三列全部出自同一事实源（ADR-044）。
// 未设环境变量时打印说明后直接通过（CI 不受影响）。

/// 汇总多个 WAL 的时延样本（跨会话合并——P99 是全量口径，不是单会话口径）。
fn merge_samples(roots: &[std::path::PathBuf]) -> (Vec<u64>, u64, u64) {
    let mut samples: Vec<u64> = Vec::new();
    let mut turns = 0u64;
    let mut fallbacks = 0u64;
    for wal in roots {
        let Ok(store) = crate::symbio_core::EventWalStore::open(wal) else {
            println!("⚠ 无法打开 {}（跳过）", wal.display());
            continue;
        };
        let snapshot = store.range(Seq::new(0));
        let latency = slo_report()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value;
        for t in latency.by_tier.values() {
            samples.extend_from_slice(&t.samples);
        }
        let fr = fallback_rate()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value;
        for st in fr.by_tier.values() {
            turns += st.turns;
            fallbacks += st.fallbacks;
        }
    }
    samples.sort_unstable();
    (samples, turns, fallbacks)
}

fn collect_wals(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_wals(&path, out);
        } else if path.file_name().is_some_and(|n| n == "v2-events.wal") {
            out.push(path);
        }
    }
}

#[tokio::test]
#[ignore = "阶段三扫描口：需 SYMBIO_SLO_SCAN_ROOTS（见上方用法说明）"]
async fn slo_scan_wal_roots() {
    let Some(spec) = std::env::var("SYMBIO_SLO_SCAN_ROOTS")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        println!("未设 SYMBIO_SLO_SCAN_ROOTS——跳过扫描（用法见 slo.test.rs 头注释）");
        return;
    };
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for dir in spec.split(';').filter(|s| !s.is_empty()) {
        collect_wals(std::path::Path::new(dir), &mut roots);
    }
    if roots.is_empty() {
        println!("给定根下没有 v2-events.wal——桥还没有真实流量可扫（先跑会话）");
        return;
    }
    println!(
        "═══ SLO 扫描（{} 个 WAL，ADR-044 同一事实源）═══",
        roots.len()
    );

    let (samples, turns, fallbacks) = merge_samples(&roots);
    if samples.is_empty() {
        println!("有 WAL 但没有 final 样本（可能全是中止/空轮）");
        return;
    }
    let all = TierLatency {
        tier: "all".into(),
        samples,
    };
    let rate = if turns == 0 {
        0.0
    } else {
        fallbacks as f64 / turns as f64
    };
    println!(
        "时延（成功轮）：P50 = {}ms · P95 = {}ms · P99 = {}ms · max = {}ms（{} 样本）",
        all.p50(),
        all.p95(),
        all.p99(),
        all.percentile(100),
        all.count()
    );
    println!("兜底率：{fallbacks}/{turns} = {:.2}%", rate * 100.0);
    println!("（成本台账逐会话差异大，按需对各 WAL 单独跑 cost_ledger——三列同源，口径见 ADR-044）");
}
