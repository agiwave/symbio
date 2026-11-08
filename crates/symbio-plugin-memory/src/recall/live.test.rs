//! 活集折叠的四条口径（as-of / 排除式遗忘 / 巩固产物 / 认知归属人）。
//!
//! 这四条原来由 `loop` 的 `recall` 投影钉（`recall.test.rs` 的 as-of 与主体隔离两条、
//! `memory_events.test.rs` 的跨标签与排除式遗忘几条）。投影随能力迁走后，判据跟着
//! 判据的 owner 走——**不是删掉重写**，是把同一批断言搬到折叠上。
//!
//! 这里没有一条是「投影不再存在」的同义反复：每条都指向一个具体的静默失效形态
//! （后台巩固污染在途视图 / 遗忘变成删除 / 巩固产物召不回 / 别人的记忆漏进来）。

use super::*;
use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::{Entity, Event, EventStore, Seq, Store, Verb, PRINCIPAL_USER};

use crate::decl::EVENT_MEMORY_FORGOTTEN;

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

fn consolidate_and_forget(store: &EventStore, source: u64, ts: i64, digest: &str) {
    store
        .append(
            Event::pending(
                format!("c-{source}"),
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                PRINCIPAL_USER,
            )
            .with_produced_by(source)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": digest,
                "tag": "经验",
                "generation": 1,
                "fidelity": 0.9,
            })),
        )
        .expect("巩固产物入格");
    store
        .append(
            Event::pending(
                format!("f-{source}"),
                EVENT_MEMORY_FORGOTTEN,
                Entity::Memory,
                Verb::Closed,
                PRINCIPAL_USER,
            )
            .with_produced_by(source)
            .with_ts(ts),
        )
        .expect("遗忘格入格");
}

fn contents(entries: &[LiveEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.content.as_str()).collect()
}

/// 新近在前：`ts` 降序、平手 `seq` 降序（两个读者都要的次序在这里定一次）。
#[test]
fn the_live_set_is_newest_first_and_deterministic() {
    let store = EventStore::new();
    encode(&store, "old", "旧的一条", "经验", 100);
    encode(&store, "tie-a", "平手甲", "经验", 500);
    encode(&store, "tie-b", "平手乙", "判断", 500);

    let events = store.range(Seq::new(0));
    let entries = live(&events, i64::MAX);
    assert_eq!(
        contents(&entries),
        vec!["平手乙", "平手甲", "旧的一条"],
        "ts 降序；同 ts 由 seq 降序收口（N1：同一输入 ⇒ 同一输出）"
    );
    // 两次求值逐字相同（折叠是纯函数）。
    assert_eq!(entries, live(&events, i64::MAX));
}

/// 标签不是折叠的闸：活集交出**全部**标签，按标签取舍是读方的事。
///
/// 这条钉的是「读侧过滤**可逆**」的机制面（DE-040 §5.4）：折叠不丢任何一条，
/// 因此读方换个标签就能换一批——若是写侧过滤，放宽条件也召不回。
#[test]
fn every_tag_stays_in_the_live_set_so_the_reader_picks() {
    let store = EventStore::new();
    encode(&store, "m0", "起床先喝水", "经验", 100);
    encode(&store, "m1", "先写正向用例", "技能", 200);
    encode(&store, "m2", "这个方案可行", "判断", 300);
    let snapshot = store.range(Seq::new(0));

    let entries = live(&snapshot, i64::MAX);
    let tags: Vec<&str> = entries.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(
        tags,
        vec!["判断", "技能", "经验"],
        "三个标签都在，次序只由新近度定"
    );
    assert_eq!(
        store.range(Seq::new(0)),
        snapshot,
        "读侧选择不得写任何东西：折叠前后事实源必须逐字节相同"
    );
}

/// 排除式遗忘：被遗忘的源不在活集里，但**事实仍在 Log**（可撤销、可审计）。
#[test]
fn forgotten_sources_leave_the_live_set_but_stay_in_the_log() {
    let store = EventStore::new();
    let source = encode(&store, "m0", "用户偏好简洁答复", "经验", 1_000);
    encode(&store, "m1", "项目用 Rust 2021", "经验", 2_000);

    let before = live(&store.range(Seq::new(0)), i64::MAX);
    assert_eq!(before.len(), 2);

    consolidate_and_forget(&store, source, 5_000, "压缩摘要");

    let after = live(&store.range(Seq::new(0)), i64::MAX);
    let after_contents = contents(&after);
    assert!(
        !after_contents.contains(&"用户偏好简洁答复"),
        "遗忘 = 折叠不再包含它：{after_contents:?}"
    );
    assert!(
        after_contents.contains(&"压缩摘要"),
        "巩固产物同样可召回——它也是一条记忆，只是 generation > 0"
    );
    assert_eq!(
        store
            .range(Seq::new(0))
            .iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED)
            .count(),
        2,
        "Log 永不删：被遗忘的编码仍在事实源里（遗忘可撤销、可审计）"
    );
}

/// as-of：`ts > now` 的事件一概不见——后台巩固不得污染在途决策（C12）。
#[test]
fn as_of_hides_events_that_have_not_happened_yet() {
    let store = EventStore::new();
    let source = encode(&store, "m0", "旧记忆", "经验", 1_000);
    let at_t = live(&store.range(Seq::new(0)), 2_000);
    assert_eq!(contents(&at_t), vec!["旧记忆"]);

    consolidate_and_forget(&store, source, 5_000, "新摘要");

    let events = store.range(Seq::new(0));
    assert_eq!(
        live(&events, 2_000),
        at_t,
        "as-of=T 的活集不受后台巩固污染（逐字节）"
    );
    // 反向：忽略 as-of 的实现（now = MAX）必然被巩固改变——上一条判据抓得住它。
    assert_ne!(
        live(&events, i64::MAX),
        at_t,
        "忽略 as-of 的实现会看见巩固产物与遗忘，判据因此可捕获"
    );
}

/// 认知归属人：只折 `actor == PRINCIPAL_USER` 的记忆。
///
/// 记忆事件的 actor 是**认知归属人**，不是跑轮主体；跨主体收放（`thread_private` /
/// `shared` / `public`）是宿主读侧闸的事（I-8），不在折叠里判——这里只挡「不是记忆
/// 归属人写的那些」。
#[test]
fn only_the_cognitive_owner_is_folded() {
    let store = EventStore::new();
    encode(&store, "mine", "我记下的", "经验", 1_000);
    store
        .append(
            Event::pending(
                "someone-else".to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                "session/other",
            )
            .with_ts(2_000)
            .with_payload(serde_json::json!({ "content": "别人记下的", "tag": "经验" })),
        )
        .expect("种子入格");

    let entries = live(&store.range(Seq::new(0)), i64::MAX);
    assert_eq!(contents(&entries), vec!["我记下的"]);
}

/// 声明视图的求值 = 同一副折叠的 JSON 读法（不是第二份实现）。
#[test]
fn the_declared_view_is_the_same_fold_serialized() {
    let store = EventStore::new();
    encode(&store, "m0", "第一条", "经验", 100);
    encode(&store, "m1", "第二条", "判断", 200);
    let events = store.range(Seq::new(0));

    let value = memory_live(&events, i64::MAX);
    let array = value.as_array().expect("活集是数组（空集也不是 null）");
    assert_eq!(array.len(), 2);
    assert_eq!(array[0]["content"], serde_json::json!("第二条"), "最新在前");
    assert_eq!(array[0]["tag"], serde_json::json!("判断"));
    assert_eq!(
        array[0]["seq"].as_u64(),
        Some(live(&events, i64::MAX)[0].seq),
        "seq 是来源事实的存储位置——巩固按它点查全史"
    );

    assert_eq!(
        memory_live(&[], i64::MAX),
        serde_json::json!([]),
        "空活集是空数组：与「视图读不出来」必须可区分"
    );
}
