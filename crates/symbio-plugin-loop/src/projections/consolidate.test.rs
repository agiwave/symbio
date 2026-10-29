//! `consolidate` 单测 —— 双约束各配正反用例（拒收 = 保真度防线真实存在）。

use super::*;

use symbio_core::{Entity, Event, EventStore, Seq, Store, Verb};

#[test]
fn authority_defaults_match_plan_01() {
    let p = ConsolidateParams::default();
    assert_eq!(p.max_gen, 3, "DE-001 §8：consolidate:max_gen = 3");
    assert!(
        (p.min_fidelity - 0.7).abs() < f64::EPSILON,
        "DE-001 §8：min_fidelity = 0.7"
    );
}

#[test]
fn healthy_consolidation_is_accepted() {
    let p = ConsolidateParams::default();
    assert_eq!(accept(&p, 1, 0.9), Ok(()));
    // 边界值：恰好等于下界与上界都放行。
    assert_eq!(accept(&p, 3, 0.7), Ok(()));
}

#[test]
fn over_generation_is_rejected() {
    let p = ConsolidateParams::default();
    assert_eq!(
        accept(&p, 4, 1.0),
        Err(Rejection::RejectedMaxGen {
            generation: 4,
            max: 3
        }),
        "代数上界防无限推进（G7）"
    );
}

#[test]
fn low_fidelity_is_rejected() {
    let p = ConsolidateParams::default();
    assert!(
        matches!(
            accept(&p, 1, 0.5),
            Err(Rejection::RejectedLowFidelity { fidelity, min }) if (fidelity - 0.5).abs() < 1e-9 && (min - 0.7).abs() < 1e-9
        ),
        "保真度下界防失真固化：0.5 < 0.7 必拒"
    );
}

/// N8：`accept` 是入库前的唯一闸门——失真巩固从未真正进 Log。
///
/// 与原 core 用例同一断言，搬到 owner 侧：判定（`accept`）与本插件的写点
/// （`memory_events`）同 crate，判据不再依赖「core 恰好也认识巩固」。
#[test]
fn consolidation_below_fidelity_floor_never_enters_the_log() {
    let params = ConsolidateParams::default();
    let store = EventStore::new();
    for (gen, fidelity) in [(1u32, 0.9f64), (1, 0.4)] {
        if accept(&params, gen, fidelity).is_ok() {
            store
                .append(
                    Event::pending(
                        format!("c-{gen}-{fidelity}"),
                        "memory.consolidated",
                        Entity::Memory,
                        Verb::Progressed,
                        0,
                        "session/main",
                    )
                    .with_produced_by(0)
                    .with_ts(2_000)
                    .with_payload(serde_json::json!({ "generation": gen, "fidelity": fidelity })),
                )
                .unwrap();
        }
    }
    let consolidated: Vec<(u32, f64)> = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == "memory.consolidated")
        .map(|e| {
            (
                e.payload["generation"].as_u64().unwrap() as u32,
                e.payload["fidelity"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(consolidated.len(), 1, "失真巩固被拒收");
    assert!(
        consolidated
            .iter()
            .all(|(g, f)| *g <= params.max_gen && *f >= params.min_fidelity),
        "{consolidated:?}"
    );
}
