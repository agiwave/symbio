//! 检索面（`Retriever`）的用例：排序的语义维、配额与降级、空读数。
//!
//! 这五条原来住 `loop` 的 `recall.test.rs`（混合打分四条 + 可降级一条）。投影随能力
//! 迁走后它们改由**面**驱动——同一个判据，换一个入口：宿主不再自己调投影，而是把
//! 可见切片交给 `retrieve_all`。

use super::*;
use symbio_plugin_api::events::EVENT_MEMORY_ENCODED;
use symbio_plugin_api::retrieval::{Retrieval, RetrievalRequest, Retriever};
use symbio_plugin_api::{Entity, Event, EventStore, Seq, Store, Verb, PRINCIPAL_USER};

fn encode(store: &EventStore, id: &str, content: &str, tag: &str, ts: i64) {
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
        .expect("种子入格");
}

/// 直接驱动本包的检索者（不经过 `retrieve_all` 的合并：本包只有一条，合并的判据在
/// `symbio-plugin-api` 的 `retrieval.test.rs`）。
fn retrieve(events: &[Event], query: Option<&str>) -> Option<Retrieval> {
    RecallRetrieval.retrieve(&RetrievalRequest { events, query })
}

fn contents(r: &Retrieval) -> Vec<&str> {
    r.entries.iter().map(|e| e.content.as_str()).collect()
}

/// 「旧但相关」的池：m0 与查询共享多个二元组但**最旧**；另两条与查询无关但更新。
fn relevance_seeded() -> (EventStore, Vec<Event>) {
    let store = EventStore::new();
    encode(
        &store,
        "old-hit",
        "部署时数据库连接池打满：把最大连接数从 20 调到 50",
        "经验",
        100,
    );
    encode(
        &store,
        "new-noise-1",
        "今天改了 README 的一段说明",
        "经验",
        900,
    );
    encode(
        &store,
        "new-noise-2",
        "调整了日志级别，减少噪声",
        "经验",
        800,
    );
    let snapshot = store.range(Seq::new(0));
    (store, snapshot)
}

/// 内容重叠最高的排第一——哪怕它是最旧的那条（纯时间序会把它埋掉）。
#[test]
fn query_ranks_content_relevant_first_even_when_oldest() {
    let (_store, snap) = relevance_seeded();

    let q = retrieve(&snap, Some("部署时数据库连接池打满的处理")).expect("有记忆");
    assert_eq!(
        q.entries.first().map(|e| e.content.as_str()),
        Some("部署时数据库连接池打满：把最大连接数从 20 调到 50"),
        "语义维是活参数：旧但相关的记忆不再输给新但无关的"
    );

    // 反向对照：同一份切片，**无查询** ⇒ 纯 ts 降序，最旧的落到最后。
    let plain = retrieve(&snap, None).expect("有记忆");
    assert_eq!(
        plain.entries.last().map(|e| e.content.as_str()),
        Some("部署时数据库连接池打满：把最大连接数从 20 调到 50"),
        "无查询是平凡值：排序与混合打分落地前逐字相同（旧记忆垫底）"
    );
    assert_ne!(
        contents(&q),
        contents(&plain),
        "同一份切片，「有查询」与「无查询」的排序必须不同——否则 query 是死参数"
    );
}

/// 换一个查询必须换一个排序。
#[test]
fn changing_the_query_changes_the_ranking() {
    let store = EventStore::new();
    encode(
        &store,
        "db",
        "部署时数据库连接池打满：调大最大连接数",
        "经验",
        100,
    );
    encode(
        &store,
        "mem",
        "用 heaptrack 定位内存泄漏，看谁的引用没释放",
        "经验",
        900,
    );
    let snap = store.range(Seq::new(0));

    let by_db = retrieve(&snap, Some("数据库连接池")).expect("有记忆");
    let by_mem = retrieve(&snap, Some("内存泄漏定位")).expect("有记忆");
    assert_eq!(
        by_db.entries[0].content,
        "部署时数据库连接池打满：调大最大连接数"
    );
    assert_eq!(
        by_mem.entries[0].content,
        "用 heaptrack 定位内存泄漏，看谁的引用没释放"
    );
    assert_ne!(contents(&by_db), contents(&by_mem));
}

/// 与全部内容零重叠的查询 ⇒ 退化成纯时间序，且一条都不许丢（I3）。
#[test]
fn unrelated_query_falls_back_to_recency_without_dropping_entries() {
    let (_store, snap) = relevance_seeded();

    let q = retrieve(&snap, Some("量子计算的纠错码")).expect("有记忆");
    assert_eq!(q.entries.len(), 3, "零重叠查询一条都不丢");
    assert_eq!(
        contents(&q),
        contents(&retrieve(&snap, None).expect("有记忆")),
        "零重叠 ⇒ 排序退化为 ts 降序（与无查询逐字相同）"
    );
}

/// 同分同 ts ⇒ 由 seq 收口（N1：同一输入 ⇒ 同一输出）。
#[test]
fn ranking_is_deterministic_when_scores_and_ts_tie() {
    let store = EventStore::new();
    encode(&store, "tie-a", "连接池太小", "经验", 500);
    encode(&store, "tie-b", "连接池太大", "经验", 500);
    let snap = store.range(Seq::new(0));

    let first = retrieve(&snap, Some("连接池")).expect("有记忆");
    let second = retrieve(&snap, Some("连接池")).expect("有记忆");
    assert_eq!(contents(&first), contents(&second));
    assert_eq!(first.entries[0].content, "连接池太大", "平手按 seq 降序");
}

/// 配额耗尽 ⇒ `degraded` + **部分结果**（DE-036 §6 验收 2：不允许空且不标降级）。
///
/// 预算语义三条自 L2b 起由本包重算（`Projection::apply` 不再经过）：配额取 `max(1)`、
/// 只截尾、窗口短于全长即降级。这条用例是那三条的执行方。
#[test]
fn exhausted_quota_degrades_with_partial_results_instead_of_silently_empty() {
    let store = EventStore::new();
    // 种满配额 + 5 条：超窗 5 条。
    for i in 0..(RECALL_SCAN_QUOTA as i64 + 5) {
        encode(
            &store,
            &format!("b{i}"),
            &format!("记忆 {i}"),
            "经验",
            100 + i,
        );
    }
    // 入参的契约是**新近在前**（宿主 `visible_slice` 排好再交）：切片顺序就是主张
    // 顺序，配额只截尾——所以这里也要先排成生产交过来的那个次序。
    let mut snap = store.range(Seq::new(0));
    snap.sort_by(|a, b| b.ts.cmp(&a.ts));

    let starved = retrieve(&snap, None).expect("有部分结果");
    assert!(starved.degraded, "超窗必须标 degraded");
    assert!(
        !starved.entries.is_empty(),
        "降级必须带部分结果——不允许空且不标降级（I3：静默超时是失败形态）"
    );
    assert_eq!(
        starved.entries.len(),
        RECALL_SCAN_QUOTA,
        "窗口只交切片头部（给定顺序即主张顺序，截尾不截头）"
    );
    // 截尾不截头：留下的是**最新**的那一批（切片是新近优先的）。
    assert_eq!(starved.entries[0].content, "记忆 504");

    // 平凡值：充足预算 ⇒ 不降级、一条不漏。
    let small = EventStore::new();
    for i in 0..5i64 {
        encode(
            &small,
            &format!("s{i}"),
            &format!("记忆 {i}"),
            "经验",
            100 + i,
        );
    }
    let full = retrieve(&small.range(Seq::new(0)), None).expect("有记忆");
    assert!(!full.degraded, "配额充足不标降级");
    assert_eq!(full.entries.len(), 5, "一条不漏");
}

/// 空切片 ⇒ `None`（与「本轮没有召回」同一个形状），不是「空读数」。
#[test]
fn no_live_memory_means_no_retrieval() {
    let store = EventStore::new();
    let snap = store.range(Seq::new(0));
    assert!(retrieve(&snap, None).is_none());
    assert!(retrieve(&snap, Some("任意查询")).is_none());

    // 只有非记忆事件同样交 `None`（折叠只认记忆两格）。
    store
        .append(Event::pending(
            "u-1".to_string(),
            "user.message",
            Entity::Scope,
            Verb::Opened,
            PRINCIPAL_USER,
        ))
        .expect("种子入格");
    assert!(retrieve(&store.range(Seq::new(0)), None).is_none());
}

/// 装配口：本包交出的检索者就是这一条，owner 与 provider 同形。
#[test]
fn the_retriever_is_submitted_under_the_plugin_id() {
    let submitted = symbio_plugin_api::retrieval::retrievers();
    assert!(
        submitted.iter().any(|r| r.owner() == "memory"),
        "检索者必须经 submit_retriever! 链接进二进制（否则宿主永远收不到读数）"
    );
}
