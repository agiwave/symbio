//! `slo` 三条视图的单测：**数字**在这里，契约在读方（`session`）。
//!
//! 与实现**同级**分文件（`decl.rs` + 同级 `decl.test.rs`）。本文件钉的是「算出来是多少」：
//! 时延只收成功收束轮、档位按**派发格**归、分位数是最近邻取法、跨会话是**样本并集后重算**
//! 而不是把三个分位数平均、兜底率的分子分母可加、跨度只收 `events ≥ 2` 的会话。
//!
//! ## 为什么手工给 `seq` / `ts`
//!
//! 真实链路里 `seq` 由存储的唯一写入口分配、`ts` 由入库时刻填。本 crate 不链接存储（也不许
//! 链接：`E-009`），所以用例在构造后手工补——测的是**折叠**，不是入库。
//!
//! ## 收束怎么归到开轮格
//!
//! 收束事实的载荷带 `turn_id`——**触发这一轮的开轮事实 id**。配对按事实身份，不按轮号：
//! 本文件里每个 `final_reply` / `fallback_reply` 的第一个「谁」参数就是那条 `user.message` 的 id。

use super::*;
use symbio_plugin_api::Seq;

const PRINCIPAL: &str = "session/main";

/// 一条开轮格（`user.message`）：本幕以哪一档跑。**档位的唯一来源。**
fn dispatch(id: &str, tier: &str, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_USER_MESSAGE,
        Entity::Scope,
        Verb::Opened,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_payload(json!({ "text": "问", "tier": tier, "principal": PRINCIPAL }));
    e.seq = Some(Seq::new(seq));
    e
}

/// 开轮格（`user.message`），不带档位。
fn user_message(id: &str, tier: Option<&str>, seq: u64, ts: i64) -> Event {
    let payload = match tier {
        Some(t) => json!({ "text": "问", "tier": t }),
        None => json!({ "text": "问" }),
    };
    let mut e = Event::pending(
        id,
        EVENT_USER_MESSAGE,
        Entity::Scope,
        Verb::Opened,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_payload(payload);
    e.seq = Some(Seq::new(seq));
    e
}

/// 成功收束（`chat.assistant.final`），带实测耗时；`turn_id` 是它所属的开轮事实 id。
fn final_reply(id: &str, turn_id: &str, cost_ms: u64, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Scope,
        Verb::Closed,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_cost_ms(cost_ms)
    .with_payload(json!({ "text": "答", "turn_id": turn_id }));
    e.seq = Some(Seq::new(seq));
    e
}

/// 兜底收束（`chat.assistant.fallback`），同样带耗时——但它**不是**时延样本。
fn fallback_reply(id: &str, turn_id: &str, cost_ms: u64, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_CHAT_ASSISTANT_FALLBACK,
        Entity::Scope,
        Verb::Closed,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_cost_ms(cost_ms)
    .with_payload(json!({ "text": "兜底", "turn_id": turn_id }));
    e.seq = Some(Seq::new(seq));
    e
}

fn rows(v: &Value) -> &Vec<Value> {
    v["by_tier"].as_array().expect("by_tier 是数组")
}

fn row<'a>(v: &'a Value, tier: &str) -> &'a Value {
    rows(v)
        .iter()
        .find(|r| r["tier"] == json!(tier))
        .unwrap_or_else(|| panic!("没有 `{tier}` 这一档：{v}"))
}

fn sample_ms(v: &Value, tier: &str) -> Vec<u64> {
    row(v, tier)["sample_ms"]
        .as_array()
        .expect("sample_ms 是数组")
        .iter()
        .filter_map(Value::as_u64)
        .collect()
}

/// 时延样本只收**成功收束**轮：兜底轮的耗时不是成功响应，混进分位数会同时污染两列。
#[test]
fn latency_takes_only_successful_finals() {
    let events = vec![
        dispatch("d1", "deep", 1, 100),
        fallback_reply("f1", "d1", 9_999, 2, 200),
        dispatch("d2", "deep", 3, 300),
        final_reply("a2", "d2", 40, 4, 340),
    ];
    let v = latency(&events, i64::MAX);
    assert_eq!(
        sample_ms(&v, "deep"),
        vec![40],
        "兜底轮的 9999ms 不该进样本"
    );
}

/// 收束归到它**指名的**那条开轮格：同一个会话里另有一条别的档位声明也不串味。
#[test]
fn latency_pairs_the_final_with_the_turn_it_names() {
    let events = vec![
        dispatch("d1", "reflex", 1, 100),
        dispatch("d2", "deep", 2, 150),
        final_reply("a1", "d2", 70, 3, 220),
    ];
    let v = latency(&events, i64::MAX);
    assert_eq!(sample_ms(&v, "deep"), vec![70], "归到它指名的那一幕");
    assert!(rows(&v).iter().all(|r| r["tier"] != json!("reflex")));
}

/// 档位读开轮格的 `tier`。
#[test]
fn latency_reads_the_tier_from_the_user_message() {
    let events = vec![
        user_message("u1", Some("fast"), 1, 100),
        final_reply("a1", "u1", 15, 2, 115),
    ];
    let v = latency(&events, i64::MAX);
    assert_eq!(sample_ms(&v, "fast"), vec![15]);
}

/// 档位缺失 / 未知 ⇒ `unspecified` 桶：**可观测，不静默归类**。
#[test]
fn an_unknown_tier_lands_in_the_unspecified_bucket() {
    let events = vec![
        dispatch("d1", "hyper", 1, 100),
        final_reply("a1", "d1", 5, 2, 105),
    ];
    let v = latency(&events, i64::MAX);
    assert_eq!(sample_ms(&v, UNSPECIFIED), vec![5]);
}

/// 分位数取法：最近邻（`ceil(p/100 × n) − 1`）。样本 10..=100 共 10 个。
#[test]
fn percentile_is_nearest_rank() {
    let events = vec![
        dispatch("d1", "deep", 1, 100),
        final_reply("a1", "d1", 10, 2, 110),
        final_reply("a2", "d1", 20, 4, 120),
        final_reply("a3", "d1", 30, 5, 130),
        final_reply("a4", "d1", 40, 6, 140),
        final_reply("a5", "d1", 50, 7, 150),
        final_reply("a6", "d1", 60, 8, 160),
        final_reply("a7", "d1", 70, 9, 170),
        final_reply("a8", "d1", 80, 10, 180),
        final_reply("a9", "d1", 90, 11, 190),
        final_reply("a10", "d1", 100, 12, 200),
    ];
    let v = latency(&events, i64::MAX);
    let r = row(&v, "deep");
    assert_eq!(r["p50"], json!(50), "n=10 ⇒ ceil(0.5×10)-1 = 4 ⇒ 第 5 个");
    assert_eq!(r["p95"], json!(100), "ceil(0.95×10)-1 = 9 ⇒ 最后一个");
    assert_eq!(r["p99"], json!(100));
}

/// 跨会话：**样本并集后重排**再取分位数——不是把两片的 p50 平均。
///
/// 两片各 `[1]` 与 `[100]`：并集后 n=2 ⇒ p50 = 第 1 个 = 1；若错成「平均分位数」
/// 会得到 50.5 或 50，两种都不是「这一档的中位数」。
#[test]
fn latency_merge_unions_samples_instead_of_averaging_percentiles() {
    let a = latency(
        &[
            dispatch("d1", "deep", 1, 100),
            final_reply("a1", "d1", 1, 2, 101),
        ],
        i64::MAX,
    );
    let b = latency(
        &[
            dispatch("d2", "deep", 1, 100),
            final_reply("a2", "d2", 100, 2, 200),
        ],
        i64::MAX,
    );
    let merged = latency_merged(&[a, b]);
    assert_eq!(sample_ms(&merged, "deep"), vec![1, 100], "并集且升序");
    assert_eq!(
        row(&merged, "deep")["p50"],
        json!(1),
        "n=2 的最近邻 P50 是第 1 个"
    );
}

/// 兜底率：分母是**所有被服务的轮**（成功 + 兜底），分子只是兜底。
#[test]
fn fallback_rate_uses_every_served_turn_as_denominator() {
    let events = vec![
        dispatch("d1", "deep", 1, 100),
        final_reply("a1", "d1", 10, 2, 110),
        dispatch("d2", "deep", 3, 200),
        fallback_reply("f2", "d2", 900, 4, 300),
    ];
    let v = fallback(&events, i64::MAX);
    let r = row(&v, "deep");
    assert_eq!(r["turns"], json!(2));
    assert_eq!(r["fallbacks"], json!(1));
    assert_eq!(r["rate"], json!(0.5));
}

/// 兜底率跨会话：分子分母相加，率由同一条算式从**汇总值**重算。
///
/// 两片各 `1/1` 与 `3/0`：相加后 `1/4 = 0.25`，不是 `(1.0 + 0.0)/2 = 0.5`。
#[test]
fn fallback_merge_adds_counts_and_recomputes_the_rate() {
    let a = fallback(
        &[
            dispatch("d1", "deep", 1, 100),
            fallback_reply("f1", "d1", 900, 2, 200),
        ],
        i64::MAX,
    );
    let b = fallback(
        &[
            dispatch("d2", "deep", 1, 100),
            final_reply("a2", "d2", 10, 2, 110),
            dispatch("d3", "deep", 3, 200),
            final_reply("a3", "d3", 10, 4, 210),
            dispatch("d4", "deep", 5, 300),
            final_reply("a4", "d4", 10, 6, 310),
        ],
        i64::MAX,
    );
    let merged = fallback_merged(&[a, b]);
    let r = row(&merged, "deep");
    assert_eq!(r["turns"], json!(4));
    assert_eq!(r["fallbacks"], json!(1));
    assert_eq!(r["rate"], json!(0.25));
}

/// 跨度 = 末事件 `ts` − 首事件 `ts`，含等待；as-of 之后的事件一概不见。
#[test]
fn span_measures_first_to_last_and_respects_as_of() {
    let events = vec![
        user_message("u1", None, 1, 1_000),
        final_reply("a1", "u1", 10, 2, 1_400),
        user_message("u2", None, 3, 9_000),
    ];
    assert_eq!(
        span(&events, 1_500)["span_ms"],
        json!(400),
        "as-of 之外的不算"
    );
    assert_eq!(span(&events, 1_500)["events"], json!(2));
    assert_eq!(span(&events, i64::MAX)["span_ms"], json!(8_000));
}

/// 跨度跨会话：只收 `events ≥ 2` 的会话；一个事件的会话会把中位数系统性往下拉。
///
/// 无样本 ⇒ `median_ms: null`——「实例里没有会话」与「会话时长 0ms」是两件事。
#[test]
fn span_merge_skips_single_event_sessions_and_reports_null_when_empty() {
    let one_event = span(&[user_message("u1", None, 1, 1_000)], i64::MAX);
    let short = span(
        &[
            user_message("u2", None, 1, 1_000),
            final_reply("a2", "u2", 10, 2, 1_200),
        ],
        i64::MAX,
    );
    let long = span(
        &[
            user_message("u3", None, 1, 1_000),
            final_reply("a3", "u3", 10, 2, 5_000),
        ],
        i64::MAX,
    );
    let merged = span_merged(&[one_event, short, long]);
    assert_eq!(merged["sessions"], json!(2), "1 事件的会话不进分母");
    // 两片的跨度是 200ms 与 4000ms；n=2 的最近邻 P50 = 第 1 个 = 200。
    assert_eq!(merged["median_ms"], json!(200));

    let empty = span_merged(&[]);
    assert_eq!(empty["sessions"], json!(0));
    assert_eq!(empty["median_ms"], Value::Null, "有据的空不是 0");
}

/// 三条视图都交两半，且 `reads` 只指它们真读的那两格。
///
#[test]
fn the_views_declare_both_halves_and_read_only_what_they_use() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert_eq!(names, vec![SLO_LATENCY, SLO_FALLBACK, SLO_SPAN]);
    for v in VIEWS {
        assert!(v.agg.is_some(), "`{}` 少了跨会话那一半（D-281）", v.name);
        assert_eq!(v.cost, COST, "切点全仓只有一个值");
        assert!(v.evidence.is_empty(), "读数不替任何事实关门");
        for (fact, field) in v.reads {
            assert!(
                (*fact, *field) == (EVENT_USER_MESSAGE, "tier"),
                "`{}` 读了意外的格：{fact}::{field}",
                v.name
            );
        }
    }
    // 跨度只读信封的 `ts`，没有载荷字段可声明。
    assert!(VIEWS[2].reads.is_empty());
}
