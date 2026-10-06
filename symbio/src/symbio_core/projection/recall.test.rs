//! `recall` 单测 —— 投影参数（**读侧过滤**）的三条性质：排除 / 可逆 / 活参数。
//!
//! 对应 [roadmap/S10 §5](../../../../docs/plan/roadmap/S10-个人认知体系注入.md) 的结论：
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
//! ## 这三条各挡一种静默失效
//!
//! | 用例 | 挡住什么 |
//! |---|---|
//! | 排除 | 过滤被改成恒真（技能会漏进只该有经验的地方） |
//! | 可逆 | 过滤被挪到**写侧**（内容不落盘，放宽阈值也召不回） |
//! | 活参数 | 参数被无视（形参还在、语义没了——最难发现的一种） |

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
    recall(VIEWER, tag.map(str::to_string))
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
