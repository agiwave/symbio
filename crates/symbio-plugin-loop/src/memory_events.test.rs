//! 记忆读侧与巩固的验收。
//!
//! **编码的判据不在这里**：经验 + 判断两条编码是情景记忆能力包的收束贡献
//! （`crates/symbio-plugin-memory`，DE-026 D-263），带溯源入格、同文去重、
//! 空发言跳过、超长截断、可见域几条判据随写方一起迁进了那个包的
//! `contributor.test.rs`。本文件剩两段各自的**出口判据**：
//!
//! - 召回：宿主那一半——跨会话扫、可见域收放、新近在前（折叠 / 排序 / 预算自 L2b
//!   起住能力包，由 `crates/symbio-plugin-memory/src/recall/` 的用例钉）；
//!   检索事实（`memory.recalled`）的判据随写方迁进能力包（`semantic` 的
//!   `contributor.test.rs`）——本文件不再判它；
//! - 巩固：**过 `accept` 才入格**——代数上界与保真度下界两个拒收分支都在
//!   生产可达；过了就写合并产物 + 排除式遗忘两条源记忆（Log 不删）。

use super::{
    consolidate, merge_with_fidelity, visible_slice, CONSOLIDATE_MIN_ENTRIES, MERGE_MAX_CHARS,
};
use crate::control_decl::guard_vocab;
use symbio_sdk::actors::SKILL_TAG;
// 分句的地基本来就住共享内核（生产与 verify 共有同一份源码，见 `merge_kernel.rs`）。
use symbio_kernel::event::{Entity, Event, EventEnvelope as _, Seq, Verb};
use symbio_kernel::store::{EventStore, Store};
use symbio_plugin_api::retrieval::{Retrieval, Retrieved};
use symbio_sdk::authz::PRINCIPAL_USER;
use symbio_sdk::invariants::check_all;
use symbio_sdk::text::merge_kernel::split_sentences;
use symbio_sdk::turn::facts::FleetSnapshot;

use crate::control_decl::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_FORGOTTEN};

/// 标签字面量：写方搬走之后，这些种子夹具需要的值是**生产值的镜像**
/// （情景记忆能力包的 `MEMORY_TAG` / `JUDGMENT_TAG`）。一致理由与
/// `memory_events.rs` 的 `CONSOLIDABLE_TAGS` 同一条：端到端判据（e2e t29）钉住，
/// 不由 crate 依赖保证。
const MEMORY_TAG: &str = "经验";
const JUDGMENT_TAG: &str = "判断";

/// 编码截断上限（写方包的同名常量镜像）：巩固的保真度判据要种出**到界**的长内容。
const ENCODE_MAX_CHARS: usize = 400;

/// 读方只吃值：一份会话的事实切片 + `own` 标记（生产上由会话存储层枚举时判定）。
fn snap(own: bool, store: &EventStore) -> FleetSnapshot {
    FleetSnapshot {
        own,
        events: store.range(Seq::new(0)),
    }
}

/// 本日志的**根事件 seq**——记忆事件的溯源必须有真实上游。
///
/// `I2` 不只要求"有边"，还要求"边不悬空"（[`check_all`] 的 C3 对偶）。
/// 夹具此前一律写 `.with_produced_by(0)`：当日志从 0 起时，那条边指向**自己**，
/// 是一处假绿——`produced_by_coverage` 只查非空，查不出自指。
/// 没有根就先种一条（`user.message` 本来就是每轮的第一条事实）。
fn root_of(store: &EventStore) -> u64 {
    match store.range(Seq::new(0)).first() {
        Some(e) => e.seq().map(|s| s.value()).unwrap_or(0),
        None => store
            .append(Event::pending(
                "root-u0",
                "user.message",
                Entity::Turn,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            ))
            .expect("根事件入格")
            .value(),
    }
}

/// 日志里的**记忆格事件**——夹具现在会先种一条根事件（用户格），
/// 于是"日志有几条"不再等于"记忆有几条"。计数断言问的是后者。
fn mem_events(store: &EventStore) -> Vec<Event> {
    store
        .range(Seq::new(0))
        .into_iter()
        .filter(|e| e.entity == Entity::Memory)
        .collect()
}

fn mem_count(store: &EventStore) -> usize {
    mem_events(store).len()
}

/// 种入一条 `memory.encoded`，**标签由调用方给**——巩固只合并同标签的记忆，
/// 要证「技能不参与经验的巩固」就得能种出别的标签。
fn seed_encoded_as(store: &EventStore, id: &str, content: &str, tag: &str, ts: i64) -> u64 {
    let src = root_of(store);
    store
        .append(
            Event::pending(
                id.to_string(),
                crate::control_decl::EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(src)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": tag,
                "generation": 0u32,
            })),
        )
        .expect("种子入格")
        .value()
}

/// 种入一条**经验**记忆（多数用例的默认标签，形态与写方一致——写方是情景记忆
/// 能力包的收束贡献，D-263）。
fn seed_encoded(store: &EventStore, id: &str, content: &str, ts: i64) -> u64 {
    seed_encoded_as(store, id, content, MEMORY_TAG, ts)
}

/// 种入一条 **`vis_scope = shared`** 的记忆（跨会话用例用）：经验类记忆的生产写方
/// 取值（写方落 `shared`）。
/// 与 [`seed_encoded`] 的差别只有载荷多一个字段——缺省（不带它）=
/// `thread_private` ⇒ 跨会话不可见，跨会话判据想看见内容必须显式种成 shared。
fn seed_shared(store: &EventStore, id: &str, content: &str, ts: i64) -> u64 {
    let src = root_of(store);
    store
        .append(
            Event::pending(
                id.to_string(),
                crate::control_decl::EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            )
            .with_produced_by(src)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": 0u32,
                "vis_scope": "shared",
            })),
        )
        .expect("种子入格")
        .value()
}

/// 种入一条 `memory.consolidated`——代数由形参给，代数上界的拒收分支非它不可达。
fn seed_consolidated(store: &EventStore, id: &str, content: &str, ts: i64, generation: u32) -> u64 {
    let src = root_of(store);
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
            .with_produced_by(src)
            .with_ts(ts)
            .with_payload(serde_json::json!({
                "content": content,
                "tag": MEMORY_TAG,
                "generation": generation,
                "fidelity": 1.0,
            })),
        )
        .expect("种子入格")
        .value()
}

fn payload_str<'a>(event: &'a Event, key: &str) -> Option<&'a str> {
    event.payload.get(key).and_then(|v| v.as_str())
}

/// 活集读数：读的是巩固读的**同一个视图**（`memory.live`，能力包交）。
///
/// 用例用它断言「合并之后哪些还在」——不另写一份折叠，否则断言的是临摹件而不是
/// 生产读到的那个读数。
fn live_reading(store: &EventStore) -> Retrieval {
    let value =
        symbio_plugin_api::read_shared_view_all(super::MEMORY_LIVE_VIEW, &store.range(Seq::new(0)))
            .expect("替身视图必须可读（能力包不在本二进制里，替身住 test_support）");
    Retrieval {
        degraded: false,
        entries: value
            .as_array()
            .expect("活集是数组")
            .iter()
            .map(|v| Retrieved {
                seq: v["seq"].as_u64().unwrap_or(0),
                content: v["content"].as_str().unwrap_or("").to_string(),
                tag: v["tag"].as_str().unwrap_or("").to_string(),
            })
            .collect(),
    }
}

/// 检索读数里有没有这条内容（测试直接读 `entries`：读数是对外形状，不再有视图类型）。
fn view_has(view: &Retrieval, content: &str) -> bool {
    view.entries.iter().any(|e| e.content == content)
}

// ==================== 巩固 ====================

/// ① 的巩固侧判据：合并产物**继承源记忆的内容侧可见域**——合并不放大
/// 可见域（源是 shared ⇒ 产物 shared；源读不到 ⇒ 落 thread_private）。
/// 写方与读方同批（纪律）：产地（encode）→ 继承（consolidate）→ 读方
/// （stats 的闸，见该处测试）三点一批，不留「有产地无人读」的半边。
#[test]
fn consolidated_memory_inherits_the_source_vis_scope() {
    let store = EventStore::new();
    // 四条 shared 的经验记忆（合并门槛 = 4）。
    for (i, text) in ["甲一说", "甲二说", "甲三说", "甲四说"].iter().enumerate() {
        store
            .append(
                Event::pending(
                    format!("sc-{i}"),
                    crate::control_decl::EVENT_MEMORY_ENCODED,
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
}

/// 巩固 · 门槛：不到 4 条不触发；到 4 条合并**最旧两条**，保真度 1，源记忆被
/// 排除式遗忘（Log 不删），投影里不再包含它们。
#[test]
fn consolidate_merges_the_two_oldest_and_forgets_the_sources() {
    let store = EventStore::new();
    // 记住每条种子自己的 seq——断言问的是"合并了哪两条"，
    // 用返回值问才不会随夹具前置事件（根事件）漂移。
    let seeded: Vec<u64> = ["起床先喝水", "周三不排会", "喝水要喝温的", "周四发周报"]
        .iter()
        .enumerate()
        .map(|(i, text)| seed_encoded(&store, &format!("e{i}"), text, 1000 + i as i64))
        .collect();
    let snap = store.range(Seq::new(0));
    assert_eq!(mem_count(&store), CONSOLIDATE_MIN_ENTRIES);

    // 只有 3 条时不动（先把第一条拿掉再问一次）。
    let three: Vec<Event> = mem_events(&store)[1..].to_vec();
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
        mem_count(&store),
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
        vec![serde_json::json!(seeded[0]), serde_json::json!(seeded[1])],
        "合并的是最旧两条（ts 1000 / 1100）"
    );
    assert_eq!(c.produced_by, Some(seeded[0]), "溯源指向最旧那条源记忆");
    assert!(c.ts > 0, "记忆事件填真实编码时刻");
    assert_eq!(payload_str(c, "tag"), Some(MEMORY_TAG));

    let forgotten: Vec<&Event> = snap2
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_FORGOTTEN)
        .collect();
    assert_eq!(forgotten.len(), 2, "两条源记忆各一条遗忘格");
    let targets: Vec<u64> = forgotten.iter().map(|e| e.produced_by.unwrap()).collect();
    assert_eq!(
        targets,
        vec![seeded[0], seeded[1]],
        "遗忘格溯源到被遗忘的那条"
    );
    for ev in &forgotten {
        assert_eq!(ev.verb, Verb::Closed, "落在 memory × closed");
        assert_eq!(
            ev.payload.get("into").and_then(|v| v.as_u64()),
            Some(merged_seq),
            "记下并进哪一条（可审计）"
        );
    }

    // 活集读数：活记忆 = 丙、丁 + 合并产物；被遗忘的不再包含（排除式遗忘）。
    let view = live_reading(&store);
    assert_eq!(view.entries.len(), 3);
    assert!(!view_has(&view, "起床先喝水"), "源记忆被遗忘");
    assert!(!view_has(&view, "周三不排会"), "源记忆被遗忘");
    assert!(view_has(&view, "喝水要喝温的"));
    assert!(view_has(&view, "周四发周报"));
    assert!(
        check_all(guard_vocab(), &snap2).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &snap2)
    );
}

/// **巩固只合并同标签的记忆**——`consolidate` 对活集读数按 `tag` 分组是**承重**的。
///
/// 技能（`tag = "skill"`）与经验同住一张网格（DE-041 §2「技能是事实，不是特殊类型」），
/// 若这个过滤退化成恒真，`consolidate` 眼里的「最旧两条」会变成**技能**：合并产物成了
/// 技能正文、两条技能被排除式遗忘——**技能静默消失，没有任何报错**。
///
/// 两条技能刻意比四条经验**更旧**：这样"选错了"必然可见（选对时它们根本不在候选里）。
/// 这也是「读侧过滤」这条设计（[DE-040 §5](../../../docs/003-design/040-个人认知体系注入.md "DE-040-个人认知体系注入")）
/// 在生产链上唯一可观察的后果——过滤本身的纯函数性质在 core 的 `recall.test.rs` 钉。
///
/// 同一条判据**复用**到「判断」标签：[`CONSOLIDABLE_TAGS`] 里两个
/// 生产写方的标签各自成组合并，技能仍在外。三条并行：经验 4 条合并、判断 4 条
/// 合并、技能 2 条不动——过滤器若对任一标签恒真，对应那条线当场红。
#[test]
fn consolidation_never_merges_across_tags() {
    let store = EventStore::new();
    seed_encoded_as(&store, "s0", "先写正向用例", SKILL_TAG, 1000);
    seed_encoded_as(&store, "s1", "先跑一遍再改", SKILL_TAG, 1100);
    let exp_seq: Vec<u64> = ["起床先喝水", "周三不排会", "喝水要喝温的", "周四发周报"]
        .iter()
        .enumerate()
        .map(|(i, text)| seed_encoded(&store, &format!("e{i}"), text, 1200 + i as i64))
        .collect();
    // 判断一组：4 条，晚于经验（各自成组，谁先谁后与判据无关）。
    let jud_seq: Vec<u64> = ["调大连接池", "先加索引", "再看慢查询", "最后复盘"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            seed_encoded_as(
                &store,
                &format!("j{i}"),
                text,
                JUDGMENT_TAG,
                1300 + i as i64,
            )
        })
        .collect();
    let snap = store.range(Seq::new(0));
    assert_eq!(mem_count(&store), 10, "2 条技能 + 4 条经验 + 4 条判断");

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
        vec![serde_json::json!(exp_seq[0]), serde_json::json!(exp_seq[1])],
        "合并的是最旧的**两条经验**（技能更旧、判断同代，但标签不同 ⇒ 不在候选里）"
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
        "判断组合并的是最旧两条判断：{merged_jud}"
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
        vec![exp_seq[0], exp_seq[1], jud_seq[0], jud_seq[1]],
        "被排除式遗忘的只有各组的源两条（经验 2 条 + 判断 2 条）"
    );

    // 技能原样留在事实源与活集里（`memory.live` 不按标签过滤 ⇒ 它们照常可召回，
    // 只是不参与**经验**的巩固；判断的源两条同理被并进组内产物）。
    let view = live_reading(&store);
    assert!(
        view_has(&view, "先写正向用例") && view_has(&view, "先跑一遍再改"),
        "技能必须还在（被遗忘 = 技能静默消失）"
    );
    assert!(
        check_all(guard_vocab(), &snap2).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &snap2)
    );
}

/// 巩固 · 保真度闸：两条都超长的记忆合并不下 ⇒ 丢掉的那一半算失真 ⇒ 低于 0.7
/// 下界 ⇒ **拒收**（源记忆原样保留，不是降标入库）。这个分支在生产上必须可达，
/// 否则 `accept` 的第二个参数就是装饰。
#[test]
fn consolidate_refuses_a_lossy_merge_and_keeps_the_sources() {
    let store = EventStore::new();
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
    assert_eq!(
        mem_count(&store),
        4,
        "拒收 = 一条都没写（源记忆原样留在 Log）"
    );

    let view = live_reading(&store);
    assert_eq!(
        view.entries.len(),
        4,
        "四条活记忆全在：拒收不丢记忆、也不降标"
    );
    assert!(view_has(&view, &"甲".repeat(ENCODE_MAX_CHARS)));
}

/// 巩固 · 代数闸（G7）：源代数 3 ⇒ 合并后代数 4 > `max_gen` ⇒ 拒收。
/// 与上一条并排要证的是**两条参数各自独立生效**——不是同一个 if 判两次。
#[test]
fn consolidate_stops_at_the_generation_ceiling() {
    let store = EventStore::new();
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
    assert_eq!(mem_count(&store), 4, "代数到顶后源记忆原样保留");
}

/// 巩固 · 代数推进：源代数 1 ⇒ 产物代数 2（读的是源事件载荷里的 `generation`，
/// 不是「数一数有几条合并产物」）。
#[test]
fn consolidate_reads_generation_from_the_source_payload() {
    let store = EventStore::new();
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
}

/// 跨代判据：连续巩固三代后，产物与**最初源**已经可分（保真度下跌），
/// 而「只与当次两条源比」会把同一产物读成 ~0.94——两个坐标系的差就是累积漂移。
#[test]
fn fidelity_is_measured_against_the_original_sources_across_generations() {
    let store = EventStore::new();
    let make = |prefix: char, n: usize| -> String {
        (0..n).map(|i| format!("{prefix}{i:02}。")).collect()
    };
    // 甲 / 乙 各 50 句（每句 4 字符）：第一代就装不下全部（留存 320/400 = 0.8），
    // 但仍在 0.7 之上——这就是后续几代累积的漂移本金。
    let (a, b) = (make('甲', 50), make('乙', 50));
    let (c, d) = (make('丙', 5), make('丁', 5));
    let (e, f, g, h) = (make('戊', 5), make('己', 5), make('庚', 5), make('辛', 5));

    for (id, text, ts) in [
        ("a", &a, 1000),
        ("b", &b, 1100),
        ("c", &c, 1200),
        ("d", &d, 1300),
    ] {
        seed_encoded(&store, id, text, ts);
    }
    let snap = store.range(Seq::new(0));
    assert_eq!(
        consolidate(&store, &snap, 1, 2000).expect("第一代").len(),
        1,
        "甲 + 乙 ⇒ 第一代产物"
    );

    for (id, text, ts) in [("e", &e, 3000), ("f", &f, 3100)] {
        seed_encoded(&store, id, text, ts);
    }
    let snap = store.range(Seq::new(0));
    assert_eq!(
        consolidate(&store, &snap, 2, 4000).expect("丙 + 丁").len(),
        1
    );

    // 此时最旧两条是 M1(2000) 与 戊(3000)：产物再次参与合并 ⇒ 第二代。
    let snap = store.range(Seq::new(0));
    assert_eq!(
        consolidate(&store, &snap, 3, 5000).expect("M1 + 戊").len(),
        1
    );

    seed_encoded(&store, "g", &g, 6000);
    seed_encoded(&store, "h", &h, 6100);
    let snap = store.range(Seq::new(0));
    assert_eq!(
        consolidate(&store, &snap, 4, 7000).expect("己 + M2").len(),
        1
    );

    let snap = store.range(Seq::new(0));
    assert_eq!(
        consolidate(&store, &snap, 5, 8000).expect("M3 + 庚").len(),
        1
    );

    let snap = store.range(Seq::new(0));
    let third = snap
        .iter()
        .find(|e| {
            e.kind == EVENT_MEMORY_CONSOLIDATED
                && e.payload.get("generation").and_then(|v| v.as_u64()) == Some(3)
        })
        .expect("写出第三代产物");
    let content = payload_str(third, "content").expect("产物有正文");
    let fidelity = third
        .payload
        .get("fidelity")
        .and_then(|v| v.as_f64())
        .expect("产物带保真度");

    // 与最初一代的并集比：甲 50 + 乙 50 + 戊 5 + 庚 5 句，只有前 80 句留存。
    let basis = [a.as_str(), b.as_str(), e.as_str(), g.as_str()];
    let sources = [content, g.as_str()];
    let (_, expected) = merge_with_fidelity(&sources, &basis, MERGE_MAX_CHARS);
    assert!(
        (fidelity - expected).abs() < 1e-9,
        "生产必须按最初源的并集算（{fidelity} vs {expected}）"
    );
    let (_, local) = merge_with_fidelity(&sources, &sources, MERGE_MAX_CHARS);
    assert!(
        local > fidelity + 0.2,
        "只与当次两条源比会把累积漂移读没：local={local} origin={fidelity}"
    );

    // 可分性：第三代产物仍带最初源的前缀，但第一代就丢掉的句子与后被挤掉的
    // 源（戊 / 庚）都不在其中——它确实已经不是最初那批源了。
    assert!(content.contains("甲00。") && content.contains("乙00。"));
    assert!(
        !content.contains("乙49。"),
        "第一代丢掉的句子不会在第三代复活"
    );
    assert!(
        !content.contains("戊00。") && !content.contains("庚00。"),
        "被上限挤掉的源与最初源可分"
    );
}

// ==================== 召回（读方） ====================
//
// 判据的分工自 L2b 起变了（DE-026 D-264）：**怎么折**（折叠 / 排序 / 扫描预算）住
// 能力包，由它的 `recall` 子模块用例钉；本文件只钉**宿主那一半**——取快照、按可见域
// 收放、交出一份「新近在前」的可见切片。所以下面五条改成直接断言 [`visible_slice`]：
// 同一批断言，只是断在判据真正住的地方（此前它们断在投影的出口上）。

/// 读方：跨会话合并、新近度降序、一份快照都没有 ⇒ 空切片。
#[test]
fn recall_view_scans_neighbour_sessions_newest_first() {
    let a = EventStore::new();
    seed_encoded(&a, "a1", "A 的早记忆", 1000);
    seed_encoded(&a, "a2", "A 的新记忆", 3000);

    let b = EventStore::new();
    seed_shared(&b, "b1", "B 的记忆", 2000);

    let slice = visible_slice(vec![snap(true, &a), snap(false, &b)], "session/main");
    let contents: Vec<&str> = slice
        .iter()
        .filter_map(|e| e.payload.get("content").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        contents,
        vec!["A 的新记忆", "B 的记忆", "A 的早记忆"],
        "跨会话合并后按新近度降序交给检索面（配额截尾 ⇒ 谁是新的一头由宿主说清楚）"
    );

    // 一份快照都没有 ⇒ 空切片（检索面据此交 `None`，调用方整段省略）。
    assert!(
        visible_slice(Vec::new(), "session/main").is_empty(),
        "没记忆就没有可见切片"
    );
}

/// 宿主把**本轮发言**交给检索面（语义维的入口）。
///
/// 排序本身不在这里判（它住能力包，见 `recall::mod.test.rs` 的
/// `query_ranks_content_relevant_first_even_when_oldest`）；这里判的是**接线**：
/// `query` 非空与为空时宿主交出去的是同一份切片、不同的查询——排序的差别由能力包
/// 决定，宿主的职责只是「把发言带过去」。
#[test]
fn recall_view_hands_the_utterance_to_the_retrieval_face() {
    let store = EventStore::new();
    seed_encoded(
        &store,
        "q-old",
        "部署时数据库连接池打满：调大最大连接数",
        1000,
    );
    seed_encoded(&store, "q-new", "今天改了 README 的一段说明", 9000);

    let plain = visible_slice(vec![snap(true, &store)], "session/main");
    let queried = visible_slice(vec![snap(true, &store)], "session/main");
    assert_eq!(
        plain, queried,
        "可见切片与查询无关：收放只按可见域，不按发言内容（语义维住能力包）"
    );
    assert_eq!(
        plain.iter().filter(|e| e.entity == Entity::Memory).count(),
        2,
        "两条记忆都在切片里——筛不筛是检索面的判断，宿主不预先替它排"
    );
}

// ==================== 读侧预过滤 ====================

/// **判断记忆不出会话**（产地的第二个消费方落地）：`thread_private` 的记忆
/// 只给**记录它的那个会话**看——别的会话的可见切片里没有它，本会话照常看得见。
/// 判断记忆用生产写方的缺省形态（载荷不带 `vis_scope` 字段 ⇒ 缺省私有），
/// 顺带钉住「存量事件按 C10 缺省收窄」。
#[test]
fn a_thread_private_memory_stays_in_its_own_session() {
    let a = EventStore::new();
    seed_encoded_as(&a, "j1", "A 的判断：先看依赖版本再升级", JUDGMENT_TAG, 1000);
    let b = EventStore::new();
    seed_shared(&b, "b1", "B 的经验", 2000);

    let slice = visible_slice(vec![snap(false, &a), snap(true, &b)], "session/main");
    let contents: Vec<&str> = slice
        .iter()
        .filter_map(|e| e.payload.get("content").and_then(|v| v.as_str()))
        .collect();
    assert!(
        contents.contains(&"B 的经验"),
        "对照组：同扫一遍，shared 的跨会话记忆照常进切片（实际: {contents:?}）"
    );
    assert!(
        !contents.iter().any(|c| c.contains("A 的判断")),
        "判断记忆不得跨会话可见（实际: {contents:?}）"
    );

    // 从 A 读：自己的判断照常在——「私有」是按线程，不是不存在。
    let own = visible_slice(vec![snap(true, &a), snap(false, &b)], "session/main");
    assert!(
        own.iter().any(|e| e
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c.contains("A 的判断"))),
        "本会话自己的判断记忆照常可见"
    );
}

/// **shared 的记忆对矩阵内主体跨会话可见**（正判据）：「人的认知可供他人借用」
/// （SC-010）在召回路径上的落地——B 会话的轮次主体 `agent:main` 读得到 A 记下的
/// 用户发言。
#[test]
fn a_shared_memory_is_recalled_across_sessions_by_matrix_members() {
    let a = EventStore::new();
    seed_shared(&a, "s1", "A 记下的共享经验", 1000);
    let b = EventStore::new();

    let slice = visible_slice(vec![snap(false, &a), snap(true, &b)], "session/main");
    assert!(
        slice.iter().any(|e| e
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c.contains("A 记下的共享经验"))),
        "矩阵内主体的跨会话切片应含他人 shared 记忆"
    );
}

/// **矩阵外主体看不见 shared**（负判据，与 stats 读闸同一分支）：
/// 未登记主体（表里没有行 ≠ 矩阵成员——`sees_of` 只认登记行）⇒ 切片里没有；
/// 属主（`user` = `SESSION_OWNER`）恒放行——两条是同一次判定的两个分支
/// （`memory_visible`），不是两个判定点。
#[test]
fn an_out_of_matrix_viewer_is_shown_no_shared_memory() {
    let a = EventStore::new();
    seed_shared(&a, "s1", "A 记下的共享经验", 1000);
    let b = EventStore::new();

    let ghost = visible_slice(vec![snap(false, &a), snap(true, &b)], "session/ghost");
    assert!(
        !ghost.iter().any(|e| e
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c.contains("A 记下的共享经验"))),
        "矩阵外主体不得经召回读到他人 shared 记忆"
    );

    // 属主恒放行：与 `stats` 读闸的「owner 相等」分支同一次判定。
    let owner = visible_slice(
        vec![snap(false, &a), snap(true, &b)],
        symbio_sdk::authz::SESSION_OWNER,
    );
    assert!(
        owner.iter().any(|e| e
            .payload
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c.contains("A 记下的共享经验"))),
        "属主（SESSION_OWNER）恒放行"
    );
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
    let dup: [&str; 2] = ["甲。乙。", "甲。乙。"];
    let (merged, fidelity) = merge_with_fidelity(&dup, &dup, MERGE_MAX_CHARS);
    assert_eq!(merged, "甲。\n乙。");
    assert_eq!(fidelity, 1.0);

    // 装不下 ⇒ 丢掉的那一半算失真（0.7 下界拦的就是它）。
    let long_a = "甲".repeat(MERGE_MAX_CHARS);
    let long_b = "乙".repeat(MERGE_MAX_CHARS);
    let long: [&str; 2] = [long_a.as_str(), long_b.as_str()];
    let (merged, fidelity) = merge_with_fidelity(&long, &long, MERGE_MAX_CHARS);
    assert!(
        merged.chars().count() <= MERGE_MAX_CHARS,
        "合并产物不超上限（按整句装，不切半行）"
    );
    assert!(
        fidelity < 0.7,
        "丢了一整条来源 ⇒ 保真度必然低于下界（实际 {fidelity}）"
    );
}

// ── 读史窗口（轮末写方的护栏）──────────────────────────────────────────────
//
// 巩固与编译各有一处「全史聚合读」。它们封在同一个窗口上：尾部 `HISTORY_WINDOW`
// 条 + 超窗时一条读数。两条用例各钉一半：**切法**（留最新、切点在下标 0）与
// **行为**（窗外更旧的记忆本轮不压缩），后者带反向——同样的四条放到窗口里
// 立刻合并，证明空结果来自窗口而不是夹具坏了。

/// 切法：超窗只留最新的一批，切的点在下标 `len - N`；刚好一窗不切。
#[test]
fn the_history_window_keeps_only_the_newest_facts() {
    let store = EventStore::new();
    let total = super::HISTORY_WINDOW + 1;
    for i in 0..total {
        store
            .append(Event::pending(
                format!("f-{i}"),
                "turn.stage",
                Entity::Turn,
                Verb::Asserted,
                0,
                PRINCIPAL_USER,
            ))
            .expect("落格");
    }
    let all = store.range(Seq::new(0));
    assert_eq!(all.len(), total, "夹具自己先超窗一条");

    let (window, cut) = super::history_window(&all);
    assert!(cut, "超窗一条 ⇒ 切过");
    assert_eq!(window.len(), super::HISTORY_WINDOW, "只留最新的一批");
    assert_eq!(
        window.first().map(|e| e.event_id.as_str()),
        all.get(1).map(|e| e.event_id.as_str()),
        "切掉的是最早的那一条（下标 0），窗口从下标 1 起"
    );

    // 刚好一窗：原样给出，且**不算切过**（读数只在真的少看时出现）。
    let (whole, cut) = super::history_window(&all[1..]);
    assert!(!cut, "刚好一窗 ⇒ 没切");
    assert_eq!(whole.len(), super::HISTORY_WINDOW);
    assert_eq!(
        whole.first().map(|e| e.event_id.as_str()),
        all.get(1).map(|e| e.event_id.as_str())
    );
}

/// 行为：比窗口更旧的记忆**本轮不压缩**（它仍在召回视图里，只是这一轮不动它）。
///
/// 这是窗口这个护栏的**后果**，写成用例是为了它不变成静默改语义：读数
/// （`plugin_debug` 里「看了多少 / 全史多少」）与这条行为是同一条判据的两面。
#[test]
fn memories_older_than_the_window_wait_for_a_later_round() {
    let store = EventStore::new();
    for (i, text) in ["起床先喝水", "周三不排会", "喝水要喝温的", "周四发周报"]
        .iter()
        .enumerate()
    {
        seed_encoded(&store, &format!("e{i}"), text, 1000 + i as i64);
    }
    // 把四条记忆推出窗口：续写一整窗无关事实（长会话的 WAL 就是这个形状）。
    for i in 0..super::HISTORY_WINDOW {
        store
            .append(Event::pending(
                format!("pad-{i}"),
                "turn.stage",
                Entity::Turn,
                Verb::Asserted,
                0,
                PRINCIPAL_USER,
            ))
            .expect("落格");
    }
    let snap = store.range(Seq::new(0));
    assert!(
        snap.len() > super::HISTORY_WINDOW,
        "夹具先超窗：{} 条",
        snap.len()
    );
    assert!(
        consolidate(&store, &snap, 7, 5000)
            .expect("巩固不失败")
            .is_empty(),
        "四条记忆都在窗外 ⇒ 本轮不压缩（不是当它们不存在，是不在这一轮动它们）"
    );

    // 反向：同样四条放回窗口里 ⇒ 立刻合并。空结果来自窗口，不是夹具坏了。
    let four = mem_events(&store);
    assert_eq!(four.len(), CONSOLIDATE_MIN_ENTRIES, "四条都还在日志里");
    assert_eq!(
        consolidate(&store, &four, 7, 5000)
            .expect("巩固不失败")
            .len(),
        1,
        "窗口覆盖到它们时 ⇒ 照常合并最旧两条"
    );
}
