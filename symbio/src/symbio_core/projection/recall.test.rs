//! `recall` 单测 —— 投影参数（**读侧过滤**）的三条性质：排除 / 可逆 / 活参数，
//! 外加混合打分（T-085 批②）的语义维判据。
//!
//! 对应 [roadmap/S10 §5](../../../../docs/003-stage/010-个人认知体系注入.md "ST-010-个人认知体系注入") 的结论：
//! 「通用性过滤」与 `memory.forgotten` 同构 ⇒ 它必须是**读侧投影参数**、不能是写侧过滤，
//! 因为只有读侧**可逆**（改参数即可召回，事实从不丢）。`tag` 是该机制**唯一已落地**的实例，
//! 而且是**承重**的：`v2_memory::consolidate` 依赖它（只合并同标签的记忆）。
//!
//! ## 为什么只到单测
//!
//! 这是**纯函数**的性质（形参进、视图出），单测直接驱动被测对象、不经过任何装配顺序
//! ——判据越确定越好。它的**生产后果**（技能不参与经验的巩固）在
//! `plugins/session/v2_memory.test.rs` 钉：那里才看得见「过滤退化成恒真」会毁掉什么。
//!
//! ## 这几条各挡一种静默失效
//!
//! | 用例 | 挡住什么 |
//! |---|---|
//! | 排除 | 过滤被改成恒真（技能会漏进只该有经验的地方） |
//! | 可逆 | 过滤被挪到**写侧**（内容不落盘，放宽阈值也召不回） |
//! | 活参数 | 参数被无视（形参还在、语义没了——最难发现的一种） |
//! | 混合打分 | 查询被收下、排序照旧（`query` 成为死参数，旧记忆永远输给新噪声） |
//! | 平凡值 | 混合打分落地把无查询路径也改窄（老调用点行为漂移） |

use super::*;
use crate::symbio_core::event::{Entity, Event, Seq, Verb, EVENT_MEMORY_ENCODED};
use crate::symbio_core::store::{EventStore, Store};

/// 记忆属主 = 视图主体（`recall` 只召回自己的，与 `tag` 是两个正交的闸）。
const VIEWER: &str = "agent:main";

/// 种一条 `memory.encoded`（`ts` 递增即新近度，投影按 `ts` 降序）。
fn encode(store: &EventStore, id: &str, content: &str, tag: &str, ts: i64) {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                VIEWER,
            )
            .with_ts(ts)
            .with_payload(serde_json::json!({ "content": content, "tag": tag })),
        )
        .expect("种子入格");
}

/// 三条**不同标签**的记忆 + 它们的快照（快照要在投影调用**之前**取，
/// 「可逆」那条判据比的就是它与调用之后的事实源）。
fn seeded() -> (EventStore, Vec<Event>) {
    let store = EventStore::new();
    encode(&store, "m0", "起床先喝水", "经验", 100);
    encode(&store, "m1", "先写正向用例", "技能", 200);
    encode(&store, "m2", "这个方案可行", "判断", 300);
    let snapshot = store.range(Seq::new(0));
    (store, snapshot)
}

fn recall_with(tag: Option<&str>, events: &[Event]) -> RecallView {
    recall(VIEWER, tag.map(str::to_string), None)
        .apply(events, i64::MAX, Budget::generous())
        .value
}

fn contents(view: &RecallView) -> Vec<&str> {
    view.entries.iter().map(|e| e.content.as_str()).collect()
}

// ── 判据一：过滤真的在排除（不是「参数收下、行为不变」）──────────────────

#[test]
fn a_tag_filter_keeps_only_its_own_tag() {
    let (_store, snap) = seeded();

    let skills = recall_with(Some("技能"), &snap);
    assert_eq!(
        contents(&skills),
        vec!["先写正向用例"],
        "只留该标签的——另两条必须被排除"
    );

    let experiences = recall_with(Some("经验"), &snap);
    assert_eq!(contents(&experiences), vec!["起床先喝水"]);

    // 不匹配任何一条 ⇒ **空视图**，不是「退化成不过滤」。
    let nothing = recall_with(Some("没有这个标签"), &snap);
    assert!(
        nothing.entries.is_empty(),
        "阈值不匹配任何一条时必须是空视图（退化成不过滤 = 过滤语义整个消失）：{:?}",
        contents(&nothing)
    );
}

// ── 判据二：可逆（S10 §5.4 的「决定性理由」）────────────────────────────

#[test]
fn widening_the_filter_recalls_what_it_excluded_without_touching_the_log() {
    let (store, snap) = seeded();

    let narrow = recall_with(Some("技能"), &snap);
    let wide = recall_with(None, &snap);

    assert_eq!(contents(&narrow).len(), 1, "窄阈值只召回一条");
    assert_eq!(contents(&wide).len(), 3, "放宽阈值 ⇒ 此前被排除的全部回来");
    assert_eq!(
        wide.entries.len(),
        snap.len(),
        "无过滤时投影一条不漏地覆盖事实源"
    );
    assert!(
        contents(&wide).contains(&"先写正向用例"),
        "被窄阈值排除过的那条必须重新可检索——这正是读侧过滤**可逆**的含义"
    );

    // 可逆的**机制**：两次调用之间事实源逐字节不变（读侧过滤只改视图，不改事实）。
    // 对照写侧过滤：内容根本没落盘，放宽阈值也召不回（S10 §5.4）。
    assert_eq!(
        store.range(Seq::new(0)),
        snap,
        "读侧过滤不得写任何东西：投影前后事实源必须逐字节相同"
    );
}

// ── 判据三：参数是活的（换值必须换结果）────────────────────────────────

#[test]
fn changing_the_tag_changes_the_view() {
    let (_store, snap) = seeded();

    let a = recall_with(Some("经验"), &snap);
    let b = recall_with(Some("判断"), &snap);
    assert_ne!(
        contents(&a),
        contents(&b),
        "换一个标签必须换一个结果（否则 `tag` 是个死参数：形参还在、语义没了）"
    );

    // 正向对照：`None` 与任一具体标签都不同——「不过滤」本身也是一个取值，
    // 不是「参数缺失」。（把 `None` 写成"忽略参数"的实现会在这里与 `a` 撞上。）
    assert_ne!(contents(&recall_with(None, &snap)), contents(&a));
}

// ── 判据四：混合打分（T-085 批②，语义维）────────────────────────────────

fn recall_query(query: &str, events: &[Event]) -> RecallView {
    recall(VIEWER, None, Some(query.to_string()))
        .apply(events, i64::MAX, Budget::generous())
        .value
}

/// 「旧但相关」的池：m0 与查询共享多个二元组但**最旧**；m1/m2 与查询无关
/// 但更新。纯时间序 ⇒ m0 排最后；内容匹配 ⇒ m0 排第一。
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

#[test]
fn query_ranks_content_relevant_first_even_when_oldest() {
    let (_store, snap) = relevance_seeded();

    let q = recall_query("部署时数据库连接池打满的处理", &snap);
    assert_eq!(
        q.entries.first().map(|e| e.content.as_str()),
        Some("部署时数据库连接池打满：把最大连接数从 20 调到 50"),
        "内容重叠最高的必须排第一——哪怕它是最旧的那条（纯时间序会把它埋掉）"
    );

    // 反向对照：同一份事实源，**无查询** ⇒ 纯 ts 降序，最旧的落到最后。
    let plain = recall_with(None, &snap);
    assert_eq!(
        plain.entries.last().map(|e| e.content.as_str()),
        Some("部署时数据库连接池打满：把最大连接数从 20 调到 50"),
        "无查询时排序必须与混合打分落地前逐字相同（旧记忆垫底）"
    );
    assert_ne!(
        contents(&q),
        contents(&plain),
        "同一事实源，「有查询」与「无查询」的排序必须不同——否则 query 是死参数"
    );
}

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

    let by_db = recall_query("数据库连接池", &snap);
    let by_mem = recall_query("内存泄漏定位", &snap);
    assert_eq!(
        by_db.entries[0].content,
        "部署时数据库连接池打满：调大最大连接数"
    );
    assert_eq!(
        by_mem.entries[0].content,
        "用 heaptrack 定位内存泄漏，看谁的引用没释放"
    );
    assert_ne!(
        contents(&by_db),
        contents(&by_mem),
        "换一个查询必须换一个排序——`query` 是活参数，不是形参装饰"
    );
}

#[test]
fn unrelated_query_falls_back_to_recency_without_dropping_entries() {
    let (_store, snap) = relevance_seeded();

    // 与全部内容零重叠的查询：重叠分全 0 ⇒ 平手判据（ts 降序）接管——
    // **必须**退化成纯时间序，且一条都不许丢（I3：不允许静默返回空）。
    let q = recall_query("量子计算的纠错码", &snap);
    assert_eq!(q.entries.len(), 3, "零重叠查询一条都不丢");
    assert_eq!(
        contents(&q),
        contents(&recall_with(None, &snap)),
        "零重叠 ⇒ 排序退化为 ts 降序（与无查询逐字相同）"
    );
}

#[test]
fn ranking_is_deterministic_when_scores_and_ts_tie() {
    let store = EventStore::new();
    // 两条同 ts、同查询词命中数（各含一次「连接池」）——平手必须由 seq 收口。
    encode(&store, "tie-a", "连接池太小", "经验", 500);
    encode(&store, "tie-b", "连接池太大", "经验", 500);
    let snap = store.range(Seq::new(0));

    let first = recall_query("连接池", &snap);
    let second = recall_query("连接池", &snap);
    assert_eq!(
        contents(&first),
        contents(&second),
        "同分同 ts 必须逐字同序（N1：同一输入 ⇒ 同一输出）"
    );
    // 平手按 seq 降序 ⇒ 后写入的 tie-b（seq 更大）在前。
    assert_eq!(first.entries[0].content, "连接池太大");
}

// ── 判据六：可降级（S06 §6 验收 2 的执行方——此前无人在 recall 侧盯它）─────

#[test]
fn exhausted_budget_degrades_with_partial_results_instead_of_silently_empty() {
    let store = EventStore::new();
    for i in 0..10i64 {
        encode(
            &store,
            &format!("b{i}"),
            &format!("记忆 {i}"),
            "经验",
            100 + i,
        );
    }
    let snap = store.range(Seq::new(0));

    // 配额 5：扫到第 6 条记忆事件即停 ⇒ 降级 + 部分结果。
    let starved = recall(VIEWER, None, None).apply(&snap, i64::MAX, Budget::new(0, 5));
    assert!(
        starved.degraded,
        "预算耗尽必须标 degraded（S06 §6 验收 2：检索预算耗尽 → 标降级）"
    );
    assert!(
        !starved.value.entries.is_empty(),
        "降级必须带部分结果——不允许空且不标降级（I3：静默超时是失败形态）"
    );
    assert!(
        starved.value.entries.len() < snap.len(),
        "部分结果：配额内的真子集"
    );
    // T-104：视图字段与投影外壳**必须一致**——外壳在 `recall_view` 取 `.value` 时被丢，
    // 视图字段是消费方唯一能读到的降级信号；两者若漂移，出口说的与实际做的不符。
    assert!(
        starved.value.degraded,
        "降级必须同时落进视图字段（T-104：视图脱离外壳流动时自描述）"
    );
    assert_eq!(
        starved.value.degraded, starved.degraded,
        "视图字段与外壳字段同源同值（一处计算、两处承载）"
    );

    // 平凡值：充足预算 ⇒ 不降级、一条不漏。
    let generous = recall(VIEWER, None, None).apply(&snap, i64::MAX, Budget::generous());
    assert!(!generous.degraded, "配额充足不标降级");
    assert_eq!(generous.value.entries.len(), snap.len(), "一条不漏");
    assert!(
        !generous.value.degraded,
        "不降级时视图字段也为假——不谎称降级（serde default 的同一口径）"
    );
}
