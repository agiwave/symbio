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
//! 收束事实的载荷带 `root_id`——**触发这一轮的开轮事实 id**。配对按事实身份，不按轮号：
//! 本文件里每个 `final_reply` / `fallback_reply` 的第一个「谁」参数就是那条 `user.message` 的 id。

use super::*;
use symbio_testkit::Seq;

const PRINCIPAL: &str = "session/main";

/// 一轮的派发格（`turn.dispatch`）：本幕以哪一档跑。**档位的唯一来源。**
///
/// `root_id` 指向这一轮的开轮事实（`user.message` 的 event id）；本文件里测试把
/// 第 1 个参数当作**轮的身份**，收束格与它同名，于是配对与生产同构。
fn dispatch(turn_id: &str, tier: &str, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        format!("{turn_id}.dispatch"),
        EVENT_TURN_DISPATCH,
        Entity::Scope,
        Verb::Asserted,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_payload(json!({ "root_id": turn_id, "tier": tier, "principal": PRINCIPAL }));
    e.seq = Some(Seq::new(seq));
    e
}

/// 开轮格（`user.message`）：兜底率的分母 / 跨度的端点。**不再带档位**（`D-282`）。
fn user_message(id: &str, seq: u64, ts: i64) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_USER_MESSAGE,
        Entity::Scope,
        Verb::Opened,
        PRINCIPAL,
    )
    .with_ts(ts)
    .with_payload(json!({ "text": "问", "id": id }));
    e.seq = Some(Seq::new(seq));
    e
}

/// 成功收束（`chat.assistant.final`），带实测耗时；`root_id` 是它所属的开轮事实 id。
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
    .with_payload(json!({ "text": "答", "root_id": turn_id }));
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
    .with_payload(json!({ "text": "兜底", "root_id": turn_id }));
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

/// 档位读派发格的 `tier`，**不读**开轮格（开轮格不再带它）。
#[test]
fn latency_reads_the_tier_from_the_dispatch_row() {
    let events = vec![
        user_message("u1", 1, 100),
        dispatch("u1", "fast", 1, 100),
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
        user_message("d1", 1, 100),
        dispatch("d1", "deep", 1, 100),
        final_reply("a1", "d1", 10, 2, 110),
        user_message("d2", 3, 200),
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
            user_message("d1", 1, 100),
            dispatch("d1", "deep", 1, 100),
            fallback_reply("f1", "d1", 900, 2, 200),
        ],
        i64::MAX,
    );
    let b = fallback(
        &[
            user_message("d2", 1, 100),
            dispatch("d2", "deep", 1, 100),
            final_reply("a2", "d2", 10, 2, 110),
            user_message("d3", 3, 200),
            dispatch("d3", "deep", 3, 200),
            final_reply("a3", "d3", 10, 4, 210),
            user_message("d4", 5, 300),
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
        user_message("u1", 1, 1_000),
        final_reply("a1", "u1", 10, 2, 1_400),
        user_message("u2", 3, 9_000),
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
    let one_event = span(&[user_message("u1", 1, 1_000)], i64::MAX);
    let short = span(
        &[
            user_message("u2", 1, 1_000),
            final_reply("a2", "u2", 10, 2, 1_200),
        ],
        i64::MAX,
    );
    let long = span(
        &[
            user_message("u3", 1, 1_000),
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
                (*fact, *field) == (EVENT_TURN_DISPATCH, "tier"),
                "`{}` 读了意外的格：{fact}::{field}",
                v.name
            );
        }
    }
    // 跨度只读信封的 `ts`，没有载荷字段可声明。
    assert!(VIEWS[2].reads.is_empty());
}

/// 报表登记：两条视图并成一行，出口行与 `fleet.by_tier` 的排版都在 owner（`D-387`）。
#[test]
fn the_tiers_report_merges_latency_and_fallback_by_tier() {
    let latency = json!({ "by_tier": [
        { "tier": "deep", "sample_ms": [900, 1200, 1500], "p50": 1200, "p95": 1500, "p99": 1500 },
        { "tier": "fast", "sample_ms": [], "p50": 0, "p95": 0, "p99": 0 },
    ]});
    let fallback = json!({ "by_tier": [
        { "tier": "deep", "turns": 4, "fallbacks": 1, "rate": 0.25 },
        { "tier": "reflect", "turns": 2, "fallbacks": 2, "rate": 1.0 },
    ]});
    let ctx = ReportCtx {
        principal: PRINCIPAL,
        cap: 0,
        readable: true,
    };

    let rows = tiers_report(&[latency.clone(), fallback.clone()], &ctx);
    let rows = rows.as_array().expect("逐档数组");
    let names: Vec<&str> = rows.iter().filter_map(|r| r["tier"].as_str()).collect();
    assert_eq!(
        names,
        vec!["deep", "fast", "reflect"],
        "两列的档位取并集并排序"
    );
    assert_eq!(rows[0]["samples"], 3, "样本数 = `sample_ms` 的长度");
    assert_eq!(rows[0]["p95"], 1500, "分位数原样搬，不在这里重算");
    assert_eq!(rows[0]["turns"], 4, "兜底列的数字原样搬");
    assert_eq!(rows[0]["rate"], 0.25);
    assert_eq!(rows[1]["turns"], 0, "只有时延样本的档位：兜底两格按 0 交");
    assert!(
        rows[0].get("sample_ms").is_none(),
        "出口不带原始样本：实例级的样本数组随会话数线性膨胀"
    );

    let fleet = tiers_fleet(&[latency, fallback], &ctx);
    assert_eq!(fleet["latency"][0]["samples"], 3);
    assert_eq!(fleet["fallback"][0]["rate"], 0.25);
    assert_eq!(fleet["fallback"][1]["tier"], "reflect");

    // 缺席（两片都读不出来 ⇒ `Null` 占位）⇒ 空表，不 panic
    assert_eq!(tiers_report(&[Value::Null, Value::Null], &ctx), json!([]));
    assert_eq!(
        tiers_fleet(&[Value::Null, Value::Null], &ctx),
        json!({ "latency": [], "fallback": [] })
    );
}
