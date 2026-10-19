//! 记忆三段验收（S5 步 11–13，[04 §3.1 批⑦](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）。
//!
//! 每段各自的**出口判据**逐条钉住：
//!
//! - 步 11 编码：写一条**带溯源、带编码时刻**的记忆；同文去重、空发言跳过、超长截断；
//! - 步 12 召回：跨会话扫、新近度降序、被遗忘的不算；注入段有空态与排版口径；
//!   检索事实溯源到触发它的那一格、同一锚幂等；
//! - 步 13 巩固：**过 `accept` 才入格**——代数上界与保真度下界两个拒收分支都在
//!   生产可达；过了就写合并产物 + 排除式遗忘两条源记忆（Log 不删）。

use super::{
    consolidate, encode, merge_with_fidelity, prompt_section, recall_view, record_recalled,
    EncodeContent, CONSOLIDATE_MIN_ENTRIES, ENCODE_MAX_CHARS, JUDGMENT_TAG, MEMORY_TAG,
    MERGE_MAX_CHARS, RECALL_DEGRADED_NOTE, RECALL_SECTION_HEAD,
};
// 分句的地基本来就住共享内核（生产与 verify 共有同一份源码，见 `merge_kernel.rs`）。
use crate::plugins::session::skill_events::SKILL_TAG;
use crate::plugins::session::store::{EventWalStore, FleetSnapshot};
use crate::symbio_core::authz::PRINCIPAL_USER;
use crate::symbio_core::merge_kernel::split_sentences;
use crate::symbio_core::{
    check_all, recall, Budget, Entity, Event, EventEnvelope as _, Seq, Store, Verb,
    EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED,
};
use std::path::{Path, PathBuf};

const V2_WAL_FILE: &str = crate::plugins::session::paths::V2_WAL_FILE;

/// 生产读方的**值入口**（存储层 `recall_snapshots` 的测试侧等价物）：
/// 扫父目录下各会话、按目录名判 `own`、读完整史。
///
/// 生产上这段在存储层；这里保留是为了让召回用例仍能按「目录布局」摆事实，
/// 同时读方本身只吃值。
fn recall_at(dir: &Path, query: &str, viewer: &str) -> Option<crate::symbio_core::RecallView> {
    let root = dir.parent().expect("会话目录有父目录");
    let own_name = dir.file_name().map(|n| n.to_owned());
    let mut snapshots = Vec::new();
    for entry in std::fs::read_dir(root).expect("扫会话根").flatten() {
        let wal = entry.path().join(V2_WAL_FILE);
        if !wal.is_file() {
            continue;
        }
        let store = EventWalStore::open_readonly(&wal).expect("事实源");
        snapshots.push(FleetSnapshot {
            own: entry.path().file_name() == own_name.as_deref(),
            events: store.range(Seq::new(0)),
        });
    }
    recall_view(snapshots, query, viewer)
}

/// 一次性临时目录（跑前先清，防上次残留污染计数断言）。
fn tmp_wal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-v2mem-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    dir.join(V2_WAL_FILE)
}

/// 种入一条 `memory.encoded`，**标签由调用方给**——巩固只合并同标签的记忆，
/// 要证「技能不参与经验的巩固」就得能种出别的标签。
fn seed_encoded_as(store: &EventWalStore, id: &str, content: &str, tag: &str, ts: i64) -> u64 {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(0)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": tag,
                "generation": 0u32,
                "vec": [],
            })),
        )
        .expect("种子入格")
        .value()
}

/// 种入一条**经验**记忆（多数用例的默认标签，形态与写方 `encode` 一致）。
fn seed_encoded(store: &EventWalStore, id: &str, content: &str, ts: i64) -> u64 {
    seed_encoded_as(store, id, content, MEMORY_TAG, ts)
}

/// 种入一条 **`vis_scope = shared`** 的记忆（[T-117](../../../../docs/009-task/117-召回跨主体读侧预过滤批.md "T-117-召回跨主体读侧预过滤批")
/// 的跨会话用例用）：经验类记忆的生产写方取值（`encode` 落 `shared`）。
/// 与 [`seed_encoded`] 的差别只有载荷多一个字段——缺省（不带它）=
/// `thread_private` ⇒ 跨会话不可见，跨会话判据想看见内容必须显式种成 shared。
fn seed_shared(store: &EventWalStore, id: &str, content: &str, ts: i64) -> u64 {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(0)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": 0u32,
                "vec": [],
                "vis_scope": "shared",
            })),
        )
        .expect("种子入格")
        .value()
}

/// 种入一条 `memory.consolidated`——代数由形参给，代数上界的拒收分支非它不可达。
fn seed_consolidated(
    store: &EventWalStore,
    id: &str,
    content: &str,
    ts: i64,
    generation: u32,
) -> u64 {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(0)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": generation,
                "fidelity": 1.0,
                "vec": [],
            })),
        )
        .expect("种子入格")
        .value()
}

fn payload_str<'a>(event: &'a Event, key: &str) -> Option<&'a str> {
    event.payload.get(key).and_then(|v| v.as_str())
}

// ==================== 步 11 · 编码 ====================

/// 步 11 出口判据：一条记忆**带溯源 + 带编码时刻**入格（溯源覆盖 100% = S06 步 11
/// 的验收），且同文去重、空发言不记、超长截断。
#[test]
fn encode_writes_a_sourced_memory_then_dedupes_same_text() {
    let wal = tmp_wal("encode");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let t = 1_700_000_000_000i64;

    let seq = encode(
        &store,
        &store.range(Seq::new(0)),
        3,
        41,
        t,
        EncodeContent::new("你好", MEMORY_TAG, "v2m-u1-a0"),
    )
    .expect("编码不失败")
    .expect("写了记忆");
    assert_eq!(seq, 0, "首格 seq 从 0 起");

    let snap = store.range(Seq::new(0));
    let ev = &snap[0];
    assert_eq!(ev.kind, EVENT_MEMORY_ENCODED);
    assert_eq!(ev.entity, Entity::Memory);
    assert_eq!(
        ev.actor, PRINCIPAL_USER,
        "记忆属主 = 会话属主（读侧同一个人）"
    );
    assert_eq!(ev.turn, 3);
    assert_eq!(
        ev.produced_by,
        Some(41),
        "溯源指向本轮 user.message 格（I2）"
    );
    assert_eq!(ev.ts, t, "ts = 编码时刻（RecallEntry::ts 的契约）");
    assert_eq!(payload_str(ev, "content"), Some("你好"));
    assert_eq!(
        payload_str(ev, "tag"),
        Some(MEMORY_TAG),
        "标签是认知内容之一"
    );
    assert_eq!(
        ev.payload.get("generation").and_then(|v| v.as_u64()),
        Some(0)
    );
    assert!(
        ev.payload
            .get("vec")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.is_empty()),
        "本批不写 embedding：vec 恒空，内容在 content"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    // 同文去重（判定方 = core 的 `contains_content`）、空白不记。
    assert_eq!(
        encode(
            &store,
            &store.range(Seq::new(0)),
            3,
            42,
            t + 1,
            EncodeContent::new("你好", MEMORY_TAG, "v2m-u1-a1"),
        )
        .expect("编码不失败"),
        None,
        "同一句话不再记一条"
    );
    assert_eq!(
        encode(
            &store,
            &store.range(Seq::new(0)),
            4,
            43,
            t + 2,
            EncodeContent::new("   \n", MEMORY_TAG, "v2m-u2-a0"),
        )
        .expect("编码不失败"),
        None,
        "空发言不记"
    );
    assert_eq!(store.range(Seq::new(0)).len(), 1, "去重期间只写了一格");

    // 不同内容 ⇒ 记；超长 ⇒ 按字符截断（记忆是提炼，不是转写副本）。
    let long = "问".repeat(ENCODE_MAX_CHARS + 100);
    encode(
        &store,
        &store.range(Seq::new(0)),
        5,
        44,
        t + 3,
        EncodeContent::new(&long, MEMORY_TAG, "v2m-u3-a0"),
    )
    .expect("编码不失败")
    .expect("不同内容要记");
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 2);
    assert_eq!(
        payload_str(&snap[1], "content").map(|s| s.chars().count()),
        Some(ENCODE_MAX_CHARS),
        "截断按字符数（不切 UTF-8 字节）"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 步 11b（T-085 批④）：智能体收束正文 → tag=`判断` 的记忆——S10 七类认知内容
/// 里 `recall:tag=judgment` 的**生产写方**（此前参数在、值缺写方）。
///
/// 与经验那条同文去重 / 同截断 / 同溯源（本轮开口格）；空正文不记。
#[test]
fn judgment_encode_writes_the_agents_conclusion_under_its_own_tag() {
    let wal = tmp_wal("judgment");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let t = 1_700_000_000_000i64;

    let seq = encode(
        &store,
        &store.range(Seq::new(0)),
        3,
        41,
        t,
        EncodeContent::new("连接池调到 50 并加了索引", JUDGMENT_TAG, "v2j-u1-a0"),
    )
    .expect("编码不失败")
    .expect("写了记忆");
    assert_eq!(seq, 0, "首格 seq 从 0 起");

    let snap = store.range(Seq::new(0));
    let ev = &snap[0];
    assert_eq!(ev.kind, EVENT_MEMORY_ENCODED);
    assert_eq!(
        payload_str(ev, "tag"),
        Some(JUDGMENT_TAG),
        "S10 的 recall:tag=judgment 从此有生产写方"
    );
    assert_eq!(
        payload_str(ev, "content"),
        Some("连接池调到 50 并加了索引"),
        "记的是智能体自己的结论"
    );
    assert_eq!(ev.produced_by, Some(41), "溯源同指本轮开口格（I2）");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    // 同文去重跨标签：同一句已被任一标签记过 ⇒ 不写第二条。
    assert_eq!(
        encode(
            &store,
            &store.range(Seq::new(0)),
            3,
            42,
            t + 1,
            EncodeContent::new("连接池调到 50 并加了索引", MEMORY_TAG, "v2m-u1-a1"),
        )
        .expect("编码不失败"),
        None,
        "跨标签同文也算重复——同一个事实不必按标签记两遍"
    );
    // 空正文不记（兜底收束那条路根本不调本函数——response = None）。
    assert_eq!(
        encode(
            &store,
            &store.range(Seq::new(0)),
            4,
            43,
            t + 2,
            EncodeContent::new("  ", JUDGMENT_TAG, "v2j-u2-a0"),
        )
        .expect("编码不失败"),
        None,
        "空发言不记"
    );
    assert_eq!(store.range(Seq::new(0)).len(), 1, "只写了一格");

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// T-094 ① 的**生产用例**：`memory.encoded` 载荷带内容侧可见域——判据不落在
/// `governance/mod.test.rs`（那边测的是枚举与字符串映射的机制），落在写方本身：
/// 默认 `thread_private`（C10 缺省，存量行为逐字不变），`with_vis_scope` 显式
/// 给宽（生产调用点 = 经验记忆的 `shared`，见 `derived_facts::record_learning`）。
/// 空 / 同文两条早退路径同样不写格——没有产地就没有假事实。
#[test]
fn encode_payload_carries_the_content_side_vis_scope() {
    let wal = tmp_wal("vis-scope");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let t = 1_700_000_000_000i64;

    // 默认（不给 with_vis_scope）= thread_private——与 T-094 之前的载荷只差一个字段。
    encode(
        &store,
        &store.range(Seq::new(0)),
        3,
        41,
        t,
        EncodeContent::new("默认可见域的一句话", MEMORY_TAG, "v2m-vs-a0"),
    )
    .expect("编码不失败")
    .expect("写了记忆");
    // 显式 shared（人的认知可借用的书写形态）。
    encode(
        &store,
        &store.range(Seq::new(0)),
        3,
        42,
        t + 1,
        EncodeContent::new("可被借用的一句话", MEMORY_TAG, "v2m-vs-a1")
            .with_vis_scope(crate::symbio_core::VisScope::Shared),
    )
    .expect("编码不失败")
    .expect("写了记忆");

    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 2);
    assert_eq!(
        payload_str(&snap[0], "vis_scope"),
        Some("thread_private"),
        "缺省 = C10 的 thread_private（存量行为不变）"
    );
    assert_eq!(
        payload_str(&snap[1], "vis_scope"),
        Some("shared"),
        "显式给定 shared —— 内容侧产地由此可写"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    // 空正文不写格：可见域不制造空事实。
    assert_eq!(
        encode(
            &store,
            &store.range(Seq::new(0)),
            4,
            43,
            t + 2,
            EncodeContent::new("  ", MEMORY_TAG, "v2m-vs-a2")
                .with_vis_scope(crate::symbio_core::VisScope::Shared),
        )
        .expect("编码不失败"),
        None,
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

// ==================== 步 13 · 巩固 ====================

/// T-094 ① 的巩固侧判据：合并产物**继承源记忆的内容侧可见域**——合并不放大
/// 可见域（源是 shared ⇒ 产物 shared；源读不到 ⇒ 落 thread_private）。
/// 写方与读方同批（T-094 纪律）：产地（encode）→ 继承（consolidate）→ 读方
/// （stats 的闸，见该处测试）三点一批，不留「有产地无人读」的半边。
#[test]
fn consolidated_memory_inherits_the_source_vis_scope() {
    let wal = tmp_wal("consolidate-scope");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    // 四条 shared 的经验记忆（合并门槛 = 4）。
    for (i, text) in ["甲一说", "甲二说", "甲三说", "甲四说"].iter().enumerate() {
        store
            .append(
                Event::pending(
                    format!("sc-{i}"),
                    EVENT_MEMORY_ENCODED,
                    Entity::Memory,
                    Verb::Opened,
                    0,
                    PRINCIPAL_USER,
                )
                .with_produced_by(0)
                .with_ts(1000 + i as i64)
                .with_payload(serde_json::json!({
                    "content": text,
                    "tag": MEMORY_TAG,
                    "generation": 0u32,
                    "vec": [],
                    "vis_scope": "shared",
                })),
            )
            .expect("种记忆");
    }
    let snap = store.range(Seq::new(0));

    let merged = consolidate(&store, &snap, 7, 5000).expect("巩固不失败");
    assert_eq!(merged.len(), 1, "到门槛 ⇒ 合并一次");

    let snap2 = store.range(Seq::new(0));
    let c = snap2
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .expect("写了一条合并产物");
    assert_eq!(
        payload_str(c, "vis_scope"),
        Some("shared"),
        "合并继承源的可见域——不放大也不丢掉"
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 步 13 · 门槛：不到 4 条不触发；到 4 条合并**最旧两条**，保真度 1，源记忆被
/// 排除式遗忘（Log 不删），投影里不再包含它们。
#[test]
fn consolidate_merges_the_two_oldest_and_forgets_the_sources() {
    let wal = tmp_wal("consolidate");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    for (i, text) in ["起床先喝水", "周三不排会", "喝水要喝温的", "周四发周报"]
        .iter()
        .enumerate()
    {
        seed_encoded(&store, &format!("e{i}"), text, 1000 + i as i64);
    }
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), CONSOLIDATE_MIN_ENTRIES);

    // 只有 3 条时不动（先把第一条拿掉再问一次）。
    let three: Vec<Event> = snap[1..].to_vec();
    assert!(
        consolidate(&store, &three, 6, 5000)
            .expect("巩固不失败")
            .is_empty(),
        "不足 {CONSOLIDATE_MIN_ENTRIES} 条不触发"
    );

    let merged_seq = consolidate(&store, &snap, 7, 5000)
        .expect("巩固不失败")
        .first()
        .map(|(_, s)| *s)
        .expect("第 4 条到位 ⇒ 触发合并");
    let snap2 = store.range(Seq::new(0));
    assert_eq!(
        snap2.len(),
        7,
        "Log 不删：4 条源 + 1 条合并产物 + 2 条遗忘格"
    );

    let c = snap2
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .expect("写了一条合并产物");
    assert_eq!(c.seq().map(|s| s.value()), Some(merged_seq));
    assert_eq!(c.entity, Entity::Memory);
    assert_eq!(c.verb, Verb::Progressed, "落在 memory × progressed");
    assert_eq!(c.turn, 7);
    assert_eq!(
        c.payload.get("generation").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        c.payload.get("fidelity").and_then(|v| v.as_f64()),
        Some(1.0),
        "四条短记忆合并没有丢句子 ⇒ 保真度 1"
    );
    let sources = c
        .payload
        .get("sources")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        sources,
        vec![serde_json::json!(0), serde_json::json!(1)],
        "合并的是最旧两条（ts 1000 / 1100 ⇒ seq 0 / 1）"
    );
    assert_eq!(c.produced_by, Some(0), "溯源指向最旧那条源记忆");
    assert!(c.ts > 0, "记忆事件填真实编码时刻");
    assert_eq!(payload_str(c, "tag"), Some(MEMORY_TAG));

    let forgotten: Vec<&Event> = snap2
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_FORGOTTEN)
        .collect();
    assert_eq!(forgotten.len(), 2, "两条源记忆各一条遗忘格");
    let targets: Vec<u64> = forgotten.iter().map(|e| e.produced_by.unwrap()).collect();
    assert_eq!(targets, vec![0, 1], "遗忘格溯源到被遗忘的那条");
    for ev in &forgotten {
        assert_eq!(ev.verb, Verb::Closed, "落在 memory × closed");
        assert_eq!(
            ev.payload.get("into").and_then(|v| v.as_u64()),
            Some(merged_seq),
            "记下并进哪一条（可审计）"
        );
    }

    // 投影：活记忆 = 丙、丁 + 合并产物；被遗忘的不再包含（排除式遗忘）。
    let view = recall(PRINCIPAL_USER, Some(MEMORY_TAG.to_string()), None)
        .apply(&snap2, i64::MAX, Budget::generous())
        .value;
    assert_eq!(view.entries.len(), 3);
    assert!(!view.contains_content("起床先喝水"), "源记忆被遗忘");
    assert!(!view.contains_content("周三不排会"), "源记忆被遗忘");
    assert!(view.contains_content("喝水要喝温的"));
    assert!(view.contains_content("周四发周报"));
    assert!(check_all(&snap2).is_empty(), "{:?}", check_all(&snap2));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// **巩固只合并同标签的记忆**——`recall(.., Some(tag), None)` 的标签过滤是**承重**的。
///
/// 技能（`tag = "skill"`）与经验同住一张网格（S11 §2「技能是事实，不是特殊类型」），
/// 若这个过滤退化成恒真，`consolidate` 眼里的「最旧两条」会变成**技能**：合并产物成了
/// 技能正文、两条技能被排除式遗忘——**技能静默消失，没有任何报错**。
///
/// 两条技能刻意比四条经验**更旧**：这样"选错了"必然可见（选对时它们根本不在候选里）。
/// 这也是「读侧过滤」这条设计（[roadmap/S10 §5](../../../../docs/009-task/130-个人认知体系注入.md "T-130-个人认知体系注入")）
/// 在生产链上唯一可观察的后果——过滤本身的纯函数性质在 core 的 `recall.test.rs` 钉。
///
/// T-085 批④ 把同一条判据**复用**到「判断」标签：[`CONSOLIDABLE_TAGS`] 里两个
/// 生产写方的标签各自成组合并，技能仍在外。三条并行：经验 4 条合并、判断 4 条
/// 合并、技能 2 条不动——过滤器若对任一标签恒真，对应那条线当场红。
#[test]
fn consolidation_never_merges_across_tags() {
    let wal = tmp_wal("consolidate-tags");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    seed_encoded_as(&store, "s0", "先写正向用例", SKILL_TAG, 1000);
    seed_encoded_as(&store, "s1", "先跑一遍再改", SKILL_TAG, 1100);
    for (i, text) in ["起床先喝水", "周三不排会", "喝水要喝温的", "周四发周报"]
        .iter()
        .enumerate()
    {
        seed_encoded(&store, &format!("e{i}"), text, 1200 + i as i64);
    }
    // 判断一组：4 条，晚于经验（各自成组，谁先谁后与判据无关）。
    for (i, text) in ["调大连接池", "先加索引", "再看慢查询", "最后复盘"]
        .iter()
        .enumerate()
    {
        seed_encoded_as(
            &store,
            &format!("j{i}"),
            text,
            JUDGMENT_TAG,
            1300 + i as i64,
        );
    }
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 10, "2 条技能 + 4 条经验 + 4 条判断");

    let merged = consolidate(&store, &snap, 7, 5000).expect("巩固不失败");
    assert_eq!(
        merged.len(),
        2,
        "经验与判断**各自**到 4 条 ⇒ 两组各合并一次（技能 2 条不动）"
    );
    assert_eq!(merged[0].0, MEMORY_TAG, "第一组合并的是经验");
    assert_eq!(merged[1].0, JUDGMENT_TAG, "第二组合并的是判断");
    let merged_seq = merged[0].1;
    let snap2 = store.range(Seq::new(0));

    let consolidated: Vec<&Event> = snap2
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .collect();
    assert_eq!(consolidated.len(), 2, "恰好两条合并产物");
    let exp_c = consolidated
        .iter()
        .find(|e| payload_str(e, "tag") == Some(MEMORY_TAG))
        .expect("经验的合并产物");
    assert_eq!(exp_c.seq().map(|s| s.value()), Some(merged_seq));
    assert_eq!(
        exp_c
            .payload
            .get("sources")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        vec![serde_json::json!(2), serde_json::json!(3)],
        "合并的是最旧的**两条经验**（技能 seq 0 / 1 与判断 seq 6..9 更旧/同代，但标签不同 ⇒ 不在候选里）"
    );
    let merged_exp = payload_str(exp_c, "content").unwrap_or("");
    assert!(
        !merged_exp.contains("先写正向用例") && !merged_exp.contains("先跑一遍再改"),
        "技能正文不得被并进经验（过滤失效时这里会红）：{merged_exp}"
    );
    assert!(
        !merged_exp.contains("调大连接池"),
        "判断正文不得被并进经验（跨组合并会当场红）：{merged_exp}"
    );
    let jud_c = consolidated
        .iter()
        .find(|e| payload_str(e, "tag") == Some(JUDGMENT_TAG))
        .expect("判断的合并产物");
    let merged_jud = payload_str(jud_c, "content").unwrap_or("");
    assert!(
        merged_jud.contains("调大连接池") && merged_jud.contains("先加索引"),
        "判断组合并的是最旧两条判断（seq 6 / 7）：{merged_jud}"
    );
    assert!(
        !merged_jud.contains("起床先喝水"),
        "经验正文不得被并进判断：{merged_jud}"
    );

    let targets: Vec<u64> = snap2
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_FORGOTTEN)
        .map(|e| e.produced_by.expect("遗忘格带溯源"))
        .collect();
    assert_eq!(
        targets,
        vec![2, 3, 6, 7],
        "被排除式遗忘的只有各组的源两条（经验 2/3 + 判断 6/7）"
    );

    // 技能原样留在事实源与视图里（`recall_view` 不按标签过滤 ⇒ 它们照常可召回，
    // 只是不参与**经验**的巩固；判断的源两条同理被并进组内产物）。
    let view = recall(PRINCIPAL_USER, None, None)
        .apply(&snap2, i64::MAX, Budget::generous())
        .value;
    assert!(
        view.contains_content("先写正向用例") && view.contains_content("先跑一遍再改"),
        "技能必须还在（被遗忘 = 技能静默消失）"
    );
    assert!(check_all(&snap2).is_empty(), "{:?}", check_all(&snap2));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 步 13 · 保真度闸：两条都超长的记忆合并不下 ⇒ 丢掉的那一半算失真 ⇒ 低于 0.7
/// 下界 ⇒ **拒收**（源记忆原样保留，不是降标入库）。这个分支在生产上必须可达，
/// 否则 `accept` 的第二个参数就是装饰。
#[test]
fn consolidate_refuses_a_lossy_merge_and_keeps_the_sources() {
    let wal = tmp_wal("lossy");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    // 最旧的两条都是顶格长文（无句界 ⇒ 各自一个句子单元）。
    seed_encoded(&store, "long-a", &"甲".repeat(ENCODE_MAX_CHARS), 1000);
    seed_encoded(&store, "long-b", &"乙".repeat(ENCODE_MAX_CHARS), 1100);
    seed_encoded(&store, "short-c", "短的丙", 1200);
    seed_encoded(&store, "short-d", "短的丁", 1300);

    let snap = store.range(Seq::new(0));
    assert!(
        consolidate(&store, &snap, 2, 5000)
            .expect("巩固不失败")
            .is_empty(),
        "保真度不足 ⇒ 拒收"
    );
    let snap2 = store.range(Seq::new(0));
    assert_eq!(snap2.len(), 4, "拒收 = 一条都没写（源记忆原样留在 Log）");

    let view = recall(PRINCIPAL_USER, Some(MEMORY_TAG.to_string()), None)
        .apply(&snap2, i64::MAX, Budget::generous())
        .value;
    assert_eq!(
        view.entries.len(),
        4,
        "四条活记忆全在：拒收不丢记忆、也不降标"
    );
    assert!(view.contains_content(&"甲".repeat(ENCODE_MAX_CHARS)));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 步 13 · 代数闸（G7）：源代数 3 ⇒ 合并后代数 4 > `max_gen` ⇒ 拒收。
/// 与上一条并排要证的是**两条参数各自独立生效**——不是同一个 if 判两次。
#[test]
fn consolidate_stops_at_the_generation_ceiling() {
    let wal = tmp_wal("maxgen");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    seed_consolidated(&store, "g3-a", "已经到顶的一", 1000, 3);
    seed_consolidated(&store, "g3-b", "已经到顶的二", 1100, 2);
    seed_encoded(&store, "e-c", "新来的丙", 1200);
    seed_encoded(&store, "e-d", "新来的丁", 1300);

    let snap = store.range(Seq::new(0));
    assert!(
        consolidate(&store, &snap, 1, 5000)
            .expect("巩固不失败")
            .is_empty(),
        "max(3, 2) + 1 = 4 > max_gen(3) ⇒ 拒收"
    );
    assert_eq!(
        store.range(Seq::new(0)).len(),
        4,
        "代数到顶后源记忆原样保留"
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 步 13 · 代数推进：源代数 1 ⇒ 产物代数 2（读的是源事件载荷里的 `generation`，
/// 不是「数一数有几条合并产物」）。
#[test]
fn consolidate_reads_generation_from_the_source_payload() {
    let wal = tmp_wal("generation");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    seed_consolidated(&store, "g1-a", "上代合并的一", 1000, 1);
    seed_consolidated(&store, "g1-b", "上代合并的二", 1100, 1);
    seed_encoded(&store, "e-c", "新来的丙", 1200);
    seed_encoded(&store, "e-d", "新来的丁", 1300);

    let snap = store.range(Seq::new(0));
    assert!(
        !consolidate(&store, &snap, 1, 5000)
            .expect("巩固不失败")
            .is_empty(),
        "代数 1 够低，合并该放行"
    );
    let snap = store.range(Seq::new(0));
    let c = snap
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_CONSOLIDATED && e.turn == 1)
        .expect("写出第二代合并产物（种子那两条 turn=0）");
    assert_eq!(
        c.payload.get("generation").and_then(|v| v.as_u64()),
        Some(2),
        "max(1, 1) + 1 = 2"
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

// ==================== 步 12 · 召回（读方 + 注入段） ====================

/// 步 12 读方：跨会话扫、新近度降序、被遗忘的不算；一条记忆都没有 ⇒ `None`。
#[test]
fn recall_view_scans_neighbour_sessions_newest_first() {
    let root = tempfile::tempdir().expect("临时根");
    let a = root.path().join("s-a");
    let b = root.path().join("s-b");
    std::fs::create_dir_all(&a).expect("A 目录");
    std::fs::create_dir_all(&b).expect("B 目录");

    let wal_a = EventWalStore::open(a.join(V2_WAL_FILE)).expect("A 的事实源");
    seed_encoded(&wal_a, "a1", "A 的早记忆", 1000);
    seed_encoded(&wal_a, "a2", "A 的新记忆", 3000);
    let doomed = seed_encoded(&wal_a, "a3", "A 的已遗忘", 4000);
    wal_a
        .append(
            Event::pending(
                "a-f".to_string(),
                EVENT_MEMORY_FORGOTTEN,
                Entity::Memory,
                Verb::Closed,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(doomed)
            .with_ts(5000)
            .with_payload(serde_json::json!({ "why": "test" })),
        )
        .expect("遗忘格入格");

    let wal_b = EventWalStore::open(b.join(V2_WAL_FILE)).expect("B 的事实源");
    seed_shared(&wal_b, "b1", "B 的记忆", 2000);

    let view = recall_at(&a, "", "session/main").expect("两个会话都有记忆");
    let contents: Vec<&str> = view.entries.iter().map(|e| e.content.as_str()).collect();
    assert_eq!(
        contents,
        vec!["A 的新记忆", "B 的记忆", "A 的早记忆"],
        "跨会话合并后按新近度降序；被排除式遗忘的那条不在视图里"
    );

    // 一个会话都没有记忆的根 ⇒ None（调用方据此整段省略）。
    let empty_root = tempfile::tempdir().expect("空临时根");
    let none_dir = empty_root.path().join("s-none");
    std::fs::create_dir_all(&none_dir).expect("空会话目录");
    assert!(
        recall_at(&none_dir, "", "session/main").is_none(),
        "没记忆就不产出视图"
    );
}

/// 步 12 读方**消费混合打分**（T-085 批②）：本轮用户正文作 query 时，
/// 内容相关的旧记忆排到无关的新记忆前面——注入段「最新在前」的排版因此
/// 把最相关的排在模型眼前。
///
/// 这是「签名扩参数 ⇒ 读方真的在用」的生产判据：投影侧的 `query_ranks_…`
/// 钉排序本身，这里钉**读方把它接上了**（两处都绿才叫落地）。
#[test]
fn recall_view_consumes_the_query_so_relevant_old_memories_rank_first() {
    let root = tempfile::tempdir().expect("临时根");
    let a = root.path().join("s-q");
    std::fs::create_dir_all(&a).expect("会话目录");
    let wal = EventWalStore::open(a.join(V2_WAL_FILE)).expect("事实源");
    seed_encoded(
        &wal,
        "q-old",
        "部署时数据库连接池打满：调大最大连接数",
        1000,
    );
    seed_encoded(&wal, "q-new-1", "今天改了 README 的一段说明", 9000);
    seed_encoded(&wal, "q-new-2", "调整了日志级别，减少噪声", 8000);

    let view = recall_at(&a, "数据库连接池打满怎么办", "session/main").expect("有记忆");
    assert_eq!(
        view.entries
            .first()
            .map(|e| e.content.as_str())
            .expect("非空"),
        "部署时数据库连接池打满：调大最大连接数",
        "读方传了 query ⇒ 相关内容必须排第一（哪怕它最旧）"
    );

    // 空 query ⇒ 平凡值：新近度降序，与本批落地前逐字相同。
    let plain = recall_at(&a, "", "session/main").expect("有记忆");
    let plain_contents: Vec<&str> = plain.entries.iter().map(|e| e.content.as_str()).collect();
    assert_eq!(
        plain_contents,
        vec![
            "今天改了 README 的一段说明",
            "调整了日志级别，减少噪声",
            "部署时数据库连接池打满：调大最大连接数"
        ],
        "无查询 ⇒ 纯 ts 降序（旧记忆垫底）"
    );

    std::fs::remove_dir_all(a).ok();
}

// ==================== 步 12 · 读侧预过滤（[T-117](../../../../docs/009-task/117-召回跨主体读侧预过滤批.md "T-117-召回跨主体读侧预过滤批")） ====================

/// **判断记忆不出会话**（T-094 产地的第二个消费方落地）：`thread_private` 的记忆
/// 只给**记录它的那个会话**看——别的会话的召回视图里没有它，本会话照常看得见。
/// 判断记忆用生产写方的缺省形态（载荷不带 `vis_scope` 字段 ⇒ 缺省私有），
/// 顺带钉住「存量事件按 C10 缺省收窄」。
#[test]
fn a_thread_private_memory_stays_in_its_own_session() {
    let root = tempfile::tempdir().expect("临时根");
    let (a, b) = (root.path().join("s-a"), root.path().join("s-b"));
    std::fs::create_dir_all(&a).expect("A 目录");
    std::fs::create_dir_all(&b).expect("B 目录");

    let wal_a = EventWalStore::open(a.join(V2_WAL_FILE)).expect("A 的事实源");
    seed_encoded_as(
        &wal_a,
        "j1",
        "A 的判断：先看依赖版本再升级",
        JUDGMENT_TAG,
        1000,
    );
    let wal_b = EventWalStore::open(b.join(V2_WAL_FILE)).expect("B 的事实源");
    seed_shared(&wal_b, "b1", "B 的经验", 2000);

    // 从 B 读（B 自己种了记忆 ⇒ 视图非空，「看不见」只能是预过滤的功劳）：
    // B 的经验在，A 的判断不在。
    let view = recall_at(&b, "", "session/main").expect("B 有自己的记忆");
    let contents: Vec<&str> = view.entries.iter().map(|e| e.content.as_str()).collect();
    assert!(
        contents.contains(&"B 的经验"),
        "对照组：同扫一遍，shared 的跨会话记忆照常进视图（实际: {contents:?}）"
    );
    assert!(
        !contents.iter().any(|c| c.contains("A 的判断")),
        "判断记忆不得跨会话可见（T-117 要消灭的泄漏；实际: {contents:?}）"
    );

    // 从 A 读：自己的判断照常在——「私有」是按线程，不是不存在。
    let own = recall_at(&a, "", "session/main").expect("A 有自己的记忆");
    assert!(
        own.entries.iter().any(|e| e.content.contains("A 的判断")),
        "本会话自己的判断记忆照常可见"
    );

    std::fs::remove_dir_all(root.path()).ok();
}

/// **shared 的记忆对矩阵内主体跨会话可见**（正判据）：「人的认知可供他人借用」
/// （SC-010）在召回路径上的落地——B 会话的轮次主体 `agent:main` 读得到 A 记下的
/// 用户发言。
#[test]
fn a_shared_memory_is_recalled_across_sessions_by_matrix_members() {
    let root = tempfile::tempdir().expect("临时根");
    let (a, b) = (root.path().join("s-a"), root.path().join("s-b"));
    std::fs::create_dir_all(&a).expect("A 目录");
    std::fs::create_dir_all(&b).expect("B 目录");

    let wal_a = EventWalStore::open(a.join(V2_WAL_FILE)).expect("A 的事实源");
    seed_shared(&wal_a, "s1", "A 记下的共享经验", 1000);

    let view = recall_at(&b, "共享", "session/main").expect("矩阵成员读得到");
    assert!(
        view.entries
            .iter()
            .any(|e| e.content.contains("A 记下的共享经验")),
        "矩阵内主体的跨会话召回应含他人 shared 记忆"
    );

    std::fs::remove_dir_all(root.path()).ok();
}

/// **矩阵外主体看不见 shared**（负判据，与 stats 读闸同一分支）：
/// 未登记主体（派生整套能力 ≠ 矩阵成员——`sees_of` 只认登记行）⇒ 视图里没有；
/// 属主（`user` = `SESSION_OWNER`）恒放行——两条是同一次判定的两个分支
/// （`memory_visible`），不是两个判定点。
#[test]
fn an_out_of_matrix_viewer_is_shown_no_shared_memory() {
    let root = tempfile::tempdir().expect("临时根");
    let (a, b) = (root.path().join("s-a"), root.path().join("s-b"));
    std::fs::create_dir_all(&a).expect("A 目录");
    std::fs::create_dir_all(&b).expect("B 目录");

    let wal_a = EventWalStore::open(a.join(V2_WAL_FILE)).expect("A 的事实源");
    seed_shared(&wal_a, "s1", "A 记下的共享经验", 1000);

    let ghost = recall_at(&b, "共享", "session/ghost");
    assert!(
        ghost.is_none()
            || !ghost
                .unwrap()
                .entries
                .iter()
                .any(|e| e.content.contains("A 记下的共享经验")),
        "矩阵外主体不得经召回读到他人 shared 记忆"
    );

    // 属主恒放行：与 `stats` 读闸的「owner 相等」分支同一次判定。
    let owner =
        recall_at(&b, "共享", crate::symbio_core::authz::SESSION_OWNER).expect("属主读得到");
    assert!(
        owner
            .entries
            .iter()
            .any(|e| e.content.contains("A 记下的共享经验")),
        "属主（SESSION_OWNER）恒放行"
    );

    std::fs::remove_dir_all(root.path()).ok();
}

/// **扫描范围 = 平级会话**（[T-118](../../../../docs/009-task/118-召回嵌套子会话遍历裁决批.md "T-118-召回嵌套子会话遍历裁决批")
/// 裁决的判据）：嵌套子会话（`<根>/<父>/sessions/<子>/`）的记忆**不进**顶层召回。
/// 理由见 `recall_view` 的「扫描范围」节——这是刻意范围，不是漏扫；改范围先改裁决。
#[test]
fn recall_view_scan_is_one_level_deliberately() {
    let root = tempfile::tempdir().expect("临时根");
    let (a, b) = (root.path().join("s-a"), root.path().join("s-b"));
    std::fs::create_dir_all(&a).expect("A 目录");
    std::fs::create_dir_all(&b).expect("B 目录");
    // 委派子会话：WAL 住在父会话目录的 sessions/ 里（分形嵌套的存储布局）。
    let sub = root.path().join("s-p").join("sessions").join("s-c");
    std::fs::create_dir_all(&sub).expect("子会话目录");

    let wal_a = EventWalStore::open(a.join(V2_WAL_FILE)).expect("A 的事实源");
    seed_encoded(&wal_a, "a1", "A 自己的记忆", 3000);
    let wal_b = EventWalStore::open(b.join(V2_WAL_FILE)).expect("B 的事实源");
    seed_shared(&wal_b, "b1", "B 的共享经验", 2000);
    let wal_c = EventWalStore::open(sub.join(V2_WAL_FILE)).expect("子会话的事实源");
    seed_shared(&wal_c, "c1", "子会话在委派里记下的", 9000);

    let view = recall_at(&a, "", "session/main").expect("有记忆");
    let contents: Vec<&str> = view.entries.iter().map(|e| e.content.as_str()).collect();
    assert!(
        contents.contains(&"B 的共享经验"),
        "平级会话照常进扫描（对照；实际: {contents:?}）"
    );
    assert!(
        !contents.iter().any(|c| c.contains("子会话")),
        "嵌套子会话不进顶层召回（刻意范围，T-118；实际: {contents:?}）"
    );

    std::fs::remove_dir_all(root.path()).ok();
}

/// 注入段排版：抬头 + 逐条一行、跨会话同文只印一行、空视图不印空壳标题。
#[test]
fn prompt_section_prints_one_line_per_memory_and_nothing_when_empty() {
    let root = tempfile::tempdir().expect("临时根");
    for (sid, ts, text) in [
        ("s-a", 1000i64, "同文的一句"),
        ("s-b", 2000, "同文的一句"),
        ("s-a2", 3000, "另一句"),
    ] {
        let dir = root.path().join(sid);
        std::fs::create_dir_all(&dir).expect("会话目录");
        let store = EventWalStore::open(dir.join(V2_WAL_FILE)).expect("事实源");
        // 跨会话的两条种成 shared（预过滤后才谈排版去重）；本会话那条用缺省。
        if sid == "s-a2" {
            seed_encoded(&store, &format!("seed-{sid}"), text, ts);
        } else {
            seed_shared(&store, &format!("seed-{sid}"), text, ts);
        }
    }
    // `s-a2` 的目录名即会话 id，读它 = 扫全根。
    let view = recall_at(&root.path().join("s-a2"), "", "session/main").expect("有记忆");
    let section = prompt_section(&view).expect("有条目 ⇒ 有段");
    assert!(
        section.starts_with(RECALL_SECTION_HEAD),
        "抬头是唯一锚点：{section}"
    );
    assert_eq!(
        section.matches("同文的一句").count(),
        1,
        "两个会话各记了一条同样的内容 ⇒ 排版只印一行（事实仍各留一格）"
    );
    assert!(section.contains("- 「另一句」"));
    assert!(
        section.contains("不要在本轮执行"),
        "条目要说清身份：旧会话里的祈使句被新一轮照做，就是当轮任务被顶掉（实测）"
    );
    assert!(
        section.find("另一句").unwrap() < section.find("同文的一句").unwrap(),
        "最新在前"
    );

    // 空视图 ⇒ None（不印空壳标题）。
    let empty = recall(PRINCIPAL_USER, None, None)
        .apply(&[], i64::MAX, Budget::generous())
        .value;
    assert!(prompt_section(&empty).is_none());

    std::fs::remove_dir_all(root.path()).ok();
}

/// 降级标注（[T-104](../../../../docs/009-task/104-检索降级可见性批.md "T-104-检索降级可见性批")）：
/// 「只显示了一部分」必须**说得出来**——抬头的措辞先于正文；不降级时不加（不谎称降级）；
/// 降级但零条目仍整段省略（不印「降级的空壳」）。
///
/// 这是 `degraded` 从**投影口径**走到**用户眼前**的出口判据：此前读方取 `View::value`
/// 把降级丢了，静默的形态从「空且不标」变成「有部分结果且不标」——两者都是同一个静默。
#[test]
fn prompt_section_says_degraded_instead_of_pretending_completeness() {
    let store = EventWalStore::open(tmp_wal("degraded")).expect("事实源");
    for i in 0..5i64 {
        seed_encoded(&store, &format!("deg-{i}"), &format!("片段 {i}"), 100 + i);
    }
    let snap = store.range(Seq::new(0));

    // 降级视图：配额 1 ⇒ 扫到第二条即停（部分结果 + degraded）。
    let degraded_view = recall(PRINCIPAL_USER, None, None)
        .apply(&snap, i64::MAX, Budget::new(0, 1))
        .value;
    assert!(degraded_view.degraded, "判据前提：视图带降级标记");
    assert!(!degraded_view.entries.is_empty(), "降级必须带部分结果");

    let section = prompt_section(&degraded_view).expect("有条目 ⇒ 有段");
    assert!(
        section.contains(RECALL_DEGRADED_NOTE),
        "降级时如实标注（不谎称完整）：{section}"
    );
    assert!(
        section.find(RECALL_DEGRADED_NOTE).unwrap() < section.find("片段").unwrap(),
        "标注在正文之前——读的人先看到「只是一部分」"
    );

    // 反向：充足预算 ⇒ 不加标注（不谎称降级）。
    let full = recall(PRINCIPAL_USER, None, None)
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    assert!(!full.degraded, "反转前提：本视图不降级");
    let full_section = prompt_section(&full).expect("有条目 ⇒ 有段");
    assert!(
        !full_section.contains(RECALL_DEGRADED_NOTE),
        "不降级不标注：{full_section}"
    );
    assert!(
        full_section.contains("片段 4"),
        "充足预算一条不漏（标注不是常驻抬头）"
    );

    // 反向：降级但**零条目** ⇒ 仍整段省略。生产不可达（配额 ≥ 1 时先扫到一条才降级），
    // 但出口必须钉住：空却贴着「只是一部分」比空壳更糟。
    let mut empty_degraded = degraded_view;
    empty_degraded.entries.clear();
    assert!(empty_degraded.degraded, "判据前提：仍然标着降级");
    assert!(
        prompt_section(&empty_degraded).is_none(),
        "空视图不印「降级的空壳」"
    );
}

// ==================== 步 12 · 写方（检索事实） ====================

/// 步 12 写方：`memory.recalled` 溯源到**触发检索的那格事件**，载荷由视图出
/// （Translator 不造信息），同一锚幂等（WAL 的 `Duplicate` 不该在生产上冒头）。
#[test]
fn record_recalled_anchors_the_retrieval_and_is_idempotent() {
    let wal = tmp_wal("recalled");
    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let trigger = seed_encoded(&store, "m1", "记住这条", 1000);

    let view = recall(PRINCIPAL_USER, None, None)
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value;
    assert_eq!(view.entries.len(), 1);

    record_recalled(
        &store,
        &view,
        trigger,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        9000,
    )
    .expect("入格");
    let snap = store.range(Seq::new(0));
    let ev = snap
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_RECALLED)
        .expect("写了一条检索事实");
    assert_eq!(ev.entity, Entity::Memory);
    assert_eq!(ev.verb, Verb::Asserted, "落在 memory × asserted");
    assert_eq!(ev.produced_by, Some(trigger), "溯源 = 触发本次检索的事件");
    assert_eq!(ev.actor, "session/main");
    assert_eq!(
        ev.actor,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "Translator 写死的 actor 必须与本机主智能体常量同值（对齐测试）"
    );
    assert_eq!(ev.ts, 9000, "检索事实也填时刻");
    assert_eq!(ev.payload.get("found").and_then(|v| v.as_u64()), Some(1));
    assert!(payload_str(ev, "top").is_some_and(|s| s.contains("记住这条")));

    // 幂等：同一锚再落一次仍是 1 条。
    record_recalled(
        &store,
        &view,
        trigger,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        9001,
    )
    .expect("重复调用不失败");
    assert_eq!(
        store
            .range(Seq::new(0))
            .iter()
            .filter(|e| e.kind == EVENT_MEMORY_RECALLED)
            .count(),
        1,
        "同一锚只落一条"
    );
    assert!(check_all(&store.range(Seq::new(0))).is_empty());

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

// ==================== 合并算法（本模块的算法面） ====================

/// 合并算法的两条边界：句界不许把数字切坏；装不下丢的句子必须算成失真。
#[test]
fn merge_keeps_numbers_intact_and_counts_loss_as_fidelity() {
    let units = split_sentences("Version 1.5 is out. Use it.");
    assert_eq!(
        units,
        vec!["Version 1.5 is out.", "Use it."],
        "英文句号只在其后是空白时才成界（`1.5` 不许被切成两半）"
    );

    // 两份同文 ⇒ 去重后仍是完整信息 ⇒ 保真度 1.0（去重省的是位置，不是信息）。
    let (merged, fidelity) = merge_with_fidelity(&["甲。乙。", "甲。乙。"], MERGE_MAX_CHARS);
    assert_eq!(merged, "甲。\n乙。");
    assert_eq!(fidelity, 1.0);

    // 装不下 ⇒ 丢掉的那一半算失真（0.7 下界拦的就是它）。
    let long_a = "甲".repeat(MERGE_MAX_CHARS);
    let long_b = "乙".repeat(MERGE_MAX_CHARS);
    let (merged, fidelity) =
        merge_with_fidelity(&[long_a.as_str(), long_b.as_str()], MERGE_MAX_CHARS);
    assert!(
        merged.chars().count() <= MERGE_MAX_CHARS,
        "合并产物不超上限（按整句装，不切半行）"
    );
    assert!(
        fidelity < 0.7,
        "丢了一整条来源 ⇒ 保真度必然低于下界（实际 {fidelity}）"
    );
}
