//! 巩固写口的判据：**接受边界**与**触发条件**（D-268 订正 / D-387 Q3）。
//!
//! 这些用例随算法一起从会话搬来：`docs/003-design/verify/consolidation_fidelity.rs`
//! 只证模型（保真度怎么算），不覆盖生产侧「什么时候该合并、什么时候该拒收」。

use super::*;
use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::{Entity, EventStore, Seq, Store, Verb, PRINCIPAL_USER};

fn encode(store: &EventStore, id: &str, content: &str, tag: &str, ts: i64) -> u64 {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                PRINCIPAL_USER,
            )
            .with_ts(ts)
            .with_payload(serde_json::json!({ "content": content, "tag": tag })),
        )
        .expect("种子入格")
        .value()
}

/// 接受边界：代数上界是预期的终点，超了就不入格。
#[test]
fn accept_rejects_beyond_the_generation_ceiling() {
    let params = ConsolidateParams::default();
    assert_eq!(accept(&params, 3, 1.0), Ok(()), "上界本身可入格");
    assert_eq!(
        accept(&params, 4, 1.0),
        Err(Rejection::RejectedMaxGen {
            generation: 4,
            max: 3
        })
    );
}

/// 接受边界：保真度不足是「这条策略装不下它」，不是降标入库。
#[test]
fn accept_rejects_below_the_fidelity_floor() {
    let params = ConsolidateParams::default();
    assert_eq!(accept(&params, 1, 0.7), Ok(()), "下限本身可入格");
    assert_eq!(
        accept(&params, 1, 0.69),
        Err(Rejection::RejectedLowFidelity {
            fidelity: 0.69,
            min: 0.7
        })
    );
}

/// 触发条件：同标签活集不足 4 条 ⇒ 不合并（不是合并不了，是不该欠）。
#[test]
fn three_live_entries_do_not_trigger() {
    let store = EventStore::new();
    encode(&store, "m1", "甲", "经验", 100);
    encode(&store, "m2", "乙", "经验", 200);
    encode(&store, "m3", "丙", "经验", 300);
    let events = store.range(Seq::new(0));
    assert!(merge_next(&events, "经验", i64::MAX).is_none());
}

/// 触发条件：4 条 ⇒ 合并**最旧两条**（不是最新两条），产物 + 两条遗忘共三笔。
#[test]
fn four_live_entries_merge_the_oldest_two() {
    let store = EventStore::new();
    let oldest = encode(&store, "m1", "甲", "经验", 100);
    let second = encode(&store, "m2", "乙", "经验", 200);
    encode(&store, "m3", "丙", "经验", 300);
    encode(&store, "m4", "丁", "经验", 400);
    let events = store.range(Seq::new(0));

    let drafts = merge_next(&events, "经验", i64::MAX).expect("四条活集应当触发");
    assert_eq!(drafts.len(), 3, "一笔合并 + 两条遗忘");
    assert_eq!(drafts[0].kind, EVENT_MEMORY_CONSOLIDATED);
    assert_eq!(
        drafts[0].payload.get("sources"),
        Some(&serde_json::json!([oldest, second])),
        "合并的是活集最旧两条"
    );
    assert_eq!(drafts[1].kind, EVENT_MEMORY_FORGOTTEN);
    assert_eq!(drafts[2].kind, EVENT_MEMORY_FORGOTTEN);
    assert_eq!(drafts[1].key, format!("v2mf-{oldest}"));
    assert_eq!(drafts[2].key, format!("v2mf-{second}"));
}

/// 标签各算各的：判断攒到 4 条时，经验那 2 条不被顺手合并。
#[test]
fn each_tag_counts_on_its_own() {
    let store = EventStore::new();
    encode(&store, "j1", "甲", "经验", 100);
    encode(&store, "j2", "乙", "经验", 200);
    encode(&store, "p1", "丙", "判断", 300);
    encode(&store, "p2", "丁", "判断", 400);
    encode(&store, "p3", "戊", "判断", 500);
    encode(&store, "p4", "己", "判断", 600);
    let events = store.range(Seq::new(0));

    let drafts = merge_next(&events, "判断", i64::MAX).expect("判断满四条");
    assert_eq!(
        drafts[0].payload.get("tag"),
        Some(&serde_json::json!("判断"))
    );
}

/// 巩固产物占一个活集名额：4 条编码合并一次后只剩 3 条活集，不再连着合并。
#[test]
fn a_consolidated_product_keeps_the_live_count_in_step() {
    let store = EventStore::new();
    let s1 = encode(&store, "m1", "甲", "经验", 100);
    let s2 = encode(&store, "m2", "乙", "经验", 200);
    encode(&store, "m3", "丙", "经验", 300);
    encode(&store, "m4", "丁", "经验", 400);
    // 一次巩固：1 条产物 + 2 条遗忘（与生产写口的形状一致）。
    store
        .append(
            Event::pending(
                "c1".to_string(),
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                PRINCIPAL_USER,
            )
            .with_produced_by(s1)
            .with_ts(500)
            .with_payload(serde_json::json!({
                "content": "甲乙",
                "tag": "经验",
                "generation": 1,
                "fidelity": 1.0,
                "sources": [s1],
            })),
        )
        .expect("巩固产物入格");
    for target in [s1, s2] {
        store
            .append(
                Event::pending(
                    format!("f-{target}"),
                    EVENT_MEMORY_FORGOTTEN,
                    Entity::Memory,
                    Verb::Closed,
                    PRINCIPAL_USER,
                )
                .with_produced_by(target)
                .with_ts(500),
            )
            .expect("遗忘格入格");
    }
    let events = store.range(Seq::new(0));
    assert!(
        merge_next(&events, "经验", i64::MAX).is_none(),
        "4 → 3 条活集，低于阈值"
    );
}

/// 幂等：同一份快照重放，产物键逐字相同（事实源按键吸收重复）。
#[test]
fn replaying_the_same_snapshot_reuses_the_event_keys() {
    let store = EventStore::new();
    encode(&store, "m1", "甲", "经验", 100);
    encode(&store, "m2", "乙", "经验", 200);
    encode(&store, "m3", "丙", "经验", 300);
    encode(&store, "m4", "丁", "经验", 400);
    let events = store.range(Seq::new(0));

    let first = merge_next(&events, "经验", i64::MAX).expect("首跑触发");
    let second = merge_next(&events, "经验", i64::MAX).expect("重放同样触发");
    let keys = |drafts: &[FactDraft]| -> Vec<String> {
        drafts.iter().map(|draft| draft.key.clone()).collect()
    };
    assert_eq!(keys(&first), keys(&second), "同一份活集 ⇒ 同一组事件键");
}
