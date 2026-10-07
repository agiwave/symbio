//! 技能编译与路由验收（S9 步 22，[04 §3.1 批⑪](../../../../docs/plan/04-工程落地.md)）：
//! **编译 → 观测 → 校准 → 回退**的每一环都要看得见，且每条口径各配一个反向用例
//! （同 trigger 不重编译；零使用不判死、一次回退即摘；本轮没技能 ⇒ 一行 I/O 都不走）。

use super::{compile, route, take_match, SkillLlmHit, SKILL_TAG};
use crate::symbio_core::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};
use crate::symbio_core::{
    check_all, recall, Budget, Entity, Event, EventWalStore, Seq, Store, Verb,
    EVENT_ASSISTANT_FINAL, EVENT_MEMORY_ENCODED, EVENT_MEMORY_RECALLED, EVENT_USER_MESSAGE,
};
use std::path::{Path, PathBuf};

/// 一个测试的临时目录：**用前先删**，上次残留不污染本次（Windows 会复用 pid）。
///
/// 目录名是 `pid + tag`，而 `seed_turn` 往里写的是**固定事件 id**（`v2u-test-a0` …）：
/// 旧运行的 `v2-events.wal` 一旦留到 pid 被复用的这次，首个 append 就撞 `Duplicate`，
/// 表现为「门禁偶发红、单跑全绿」。删掉重建才谈得上从空开始。
fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-v2skills-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    dir
}

fn wal_of(dir: &Path) -> PathBuf {
    dir.join(super::super::paths::V2_WAL_FILE)
}

/// 一轮的**溯源锚**：`user.message` 开轮 + `assistant.final` 收束——
/// 技能的 `produced_by` 指向开轮格（I2），`check_all` 才看得见整条链是绿的。
fn seed_turn(store: &EventWalStore) -> u64 {
    let anchor = store
        .append(
            Event::pending(
                "v2u-test-a0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                crate::symbio_core::authz::PRINCIPAL_USER,
            )
            .with_payload(serde_json::json!({
                "text": "帮我写测试",
                "tier": "deep",
                "turn_ref": "u-test",
            })),
        )
        .expect("开轮格入格")
        .value();
    store
        .append(
            Event::pending(
                "v2f-test-a0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(anchor)
            .with_cost_ms(7)
            .with_payload(serde_json::json!({ "text": "好的，先写正向用例。" })),
        )
        .expect("收束格入格");
    anchor
}

/// 一条路由观测（生产写方是 `v2_bridge::record`，这里直写**同一形状**——
/// `calibration` 只认载荷里的 `skill_id` + `fallback`，格子叫什么它不管）。
fn observe(store: &EventWalStore, n: usize, skill_id: &str, fallback: bool, anchor: u64) {
    store
        .append(
            Event::pending(
                format!("v2s-test-{n}"),
                EVENT_MEMORY_RECALLED,
                Entity::Memory,
                Verb::Asserted,
                0,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(anchor)
            .with_ts(1_700_000_000_100)
            .with_payload(serde_json::json!({ "skill_id": skill_id, "fallback": fallback })),
        )
        .expect("观测入格");
}

/// 本会话当前的召回视图（技能就在里面——`tag` 由投影从载荷带出来）。
///
/// viewer 取**属主**（与生产读方 `v2_memory::recall_view` 同一个 viewer）：
/// `memory × *` 的 actor 是属主、投影按 `actor == viewer` 过滤，两个 viewer
/// 不一致时单测看得到的东西生产看不到（断链就是这么溜进来的）。
fn view_of(store: &EventWalStore) -> crate::symbio_core::RecallView {
    recall(PRINCIPAL_USER, None)
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value
}

/// 写一轮：编译出一条技能，返回它的 `skill_id`（锚 = 开轮格）。
fn seed_skill(dir: &Path, trigger: &str) -> String {
    let store = EventWalStore::open(wal_of(dir)).expect("WAL");
    let anchor = seed_turn(&store);
    let snap = store.range(Seq::new(0));
    let seq = compile(
        &store,
        &snap,
        0,
        anchor,
        trigger,
        "好的，先写正向用例。",
        1_700_000_000_000,
    )
    .expect("编译入格")
    .expect("首次必须产出技能");
    let snap = store.range(Seq::new(0));
    let skill = snap
        .iter()
        .find(|e| e.seq.map(|s| s.value()) == Some(seq))
        .expect("技能格");
    skill
        .payload
        .get("skill_id")
        .and_then(|v| v.as_str())
        .expect("skill_id")
        .to_string()
}

/// 正向：技能事件是**事实**——同一格 `memory.encoded`、溯源指向源轨迹、轮号与
/// 时间戳由写方给；同 trigger 至多一次（技能集不随收束次数无界增长）。
#[test]
fn compile_writes_a_sourced_skill_exactly_once_per_trigger() {
    let dir = tmp_dir("compile");
    let store = EventWalStore::open(wal_of(&dir)).expect("WAL");
    let anchor = seed_turn(&store);

    let snap = store.range(Seq::new(0));
    let seq = compile(
        &store,
        &snap,
        0,
        anchor,
        "帮我写测试",
        "好的，先写正向用例。",
        1_700_000_000_000,
    )
    .expect("编译入格")
    .expect("首次必须产出技能");

    let snap = store.range(Seq::new(0));
    let skill = snap
        .iter()
        .find(|e| e.seq.map(|s| s.value()) == Some(seq))
        .expect("技能格");
    assert_eq!(skill.kind, EVENT_MEMORY_ENCODED, "技能是事实，不是特殊类型");
    assert_eq!(
        skill.payload.get("tag").and_then(|v| v.as_str()),
        Some(SKILL_TAG)
    );
    assert_eq!(
        skill.payload.get("trigger").and_then(|v| v.as_str()),
        Some("帮我写测试")
    );
    assert_eq!(
        skill.produced_by,
        Some(anchor),
        "溯源指向源轨迹（I2：技能溯源 100%）"
    );
    assert_eq!(skill.turn, 0, "轮号是写方口径，不认 core 造信封时的占位");
    assert_eq!(skill.ts, 1_700_000_000_000, "ts 是编译时刻（写方给）");
    assert_eq!(
        skill.actor, PRINCIPAL_USER,
        "actor 归位属主——`memory × *` 的 actor 是属主不是作者，占位（agent:main）\
         会让召回投影按 `actor == viewer` 把技能挡在视图外（断链即红）"
    );
    let skill_id = skill
        .payload
        .get("skill_id")
        .and_then(|v| v.as_str())
        .expect("skill_id");
    assert_eq!(skill_id.len(), 16, "skill_id = trigger 的稳定 64 位哈希");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    // 反向：同 trigger（含前后空白）不再编译——否则技能随收束次数无界增长。
    let before = snap.len();
    let again = compile(
        &store,
        &snap,
        1,
        anchor,
        "  帮我写测试 ",
        "别的回答",
        1_700_000_001_000,
    )
    .expect("重复编译不报错");
    assert!(again.is_none(), "同一句 trigger 至多编译一次");
    assert_eq!(
        store.range(Seq::new(0)).len(),
        before,
        "没有新增格（幂等由 trigger 判据兼任）"
    );
}

/// 正向：**零使用的新技能不判死**（`confidence()` 无数据按 1.0）⇒ 快路、留在
/// 视图里，且观测如实记 `fallback = false`；同一次判定的第二个出口（可用集）
/// 带上命中所需的全部数据。
#[test]
fn a_new_skill_stays_in_the_prompt_and_is_observed_as_a_fast_path() {
    let dir = tmp_dir("fresh");
    let skill_id = seed_skill(&dir, "帮我写测试");

    let store = EventWalStore::open_readonly(wal_of(&dir)).expect("WAL");
    let mut view = view_of(&store);
    assert_eq!(
        view.entries.len(),
        1,
        "只召回技能本身（观测是 memory.recalled，不进召回投影）"
    );
    assert_eq!(view.entries[0].tag, SKILL_TAG);

    let routing = route(&dir, &mut view);
    assert_eq!(
        routing.obs,
        vec![(skill_id.clone(), false)],
        "零使用 ⇒ 快路"
    );
    assert_eq!(view.entries.len(), 1, "快路不摘条目");

    // 第二个出口：可用集（快路的候选）——命中判据与产物都在里面。
    assert_eq!(routing.hits.len(), 1, "判成快路的技能进可用集");
    assert_eq!(routing.hits[0].skill_id, skill_id, "可用集带技能身份");
    assert_eq!(
        routing.hits[0].trigger, "帮我写测试",
        "触发串 = 编译时的用户发言（命中判据比的就是它）"
    );
    assert_eq!(
        routing.hits[0].content, "好的，先写正向用例。",
        "技能正文 = 命中后直接收束的那段"
    );
}

/// 反向（S11 §5 静默失效第 2 行的守卫）：**一次回退**就把置信度打到阈值以下
/// ⇒ 技能被摘出本轮视图（不进提示词），观测记 `fallback = true`；它同时**不进
/// 可用集**——回退的技能连提示词都不该在，更不该被拿去执行。
#[test]
fn a_fallen_back_skill_is_dropped_from_this_turn() {
    let dir = tmp_dir("fallback");
    let skill_id = seed_skill(&dir, "帮我写测试");
    {
        let store = EventWalStore::open(wal_of(&dir)).expect("WAL");
        // 观测的溯源锚沿用开轮格：从快照里取第一条 `user.message` 的 seq。
        let anchor = store
            .range(Seq::new(0))
            .iter()
            .find(|e| e.kind == EVENT_USER_MESSAGE)
            .and_then(|e| e.seq)
            .expect("开轮格");
        observe(&store, 0, &skill_id, true, anchor.value());
    }

    let store = EventWalStore::open_readonly(wal_of(&dir)).expect("WAL");
    let mut view = view_of(&store);
    assert_eq!(view.entries.len(), 1, "技能本来在视图里");

    let routing = route(&dir, &mut view);
    assert_eq!(routing.obs, vec![(skill_id, true)], "低置信 ⇒ 回退");
    assert!(
        view.entries.is_empty(),
        "回退的作用点就是提示词：这条技能不进本轮请求"
    );
    assert!(
        routing.hits.is_empty(),
        "回退的技能不进可用集（不能既判它不可信、又拿它去收束本轮）"
    );
}

/// 反向（读侧的平凡值）：本轮没召回技能 ⇒ 直接返回，连不存在的目录都不去开
/// ——默认关掉编译时读侧恒走这一支，不成为每轮的固定开销。
#[test]
fn without_any_skill_the_reader_touches_nothing() {
    let dir = tmp_dir("noskill");
    let store = EventWalStore::open(wal_of(&dir)).expect("WAL");
    let _ = seed_turn(&store);
    let mut view = view_of(&store);
    assert!(view.entries.is_empty(), "没有技能可召回");

    let routing = route(&dir, &mut view);
    assert!(routing.obs.is_empty() && routing.hits.is_empty());
    assert!(view.entries.is_empty());
    // 目录里根本没有 WAL 的情形同样不炸（读方不创建文件）。
    let missing = tmp_dir("missing-wal");
    let routing = route(&missing, &mut view);
    assert!(routing.obs.is_empty() && routing.hits.is_empty());
}

/// 命中判据：**归一化后的逐字恒等**。
///
/// 反向即失效：归一化与写方不同源（写方存的是 `truncate_chars(user_text.trim(), N)`，
/// 读方若拿原文比），长发言**永远命不中**——而且没有任何报错，表现为"技能明明编了
/// 却从不生效"。这条判据同时钉住"更宽的『同类』不在本批"（S11 §7：那是算法问题）。
#[test]
fn the_hit_judgement_is_literal_after_the_writers_normalisation() {
    let hits = vec![SkillLlmHit {
        skill_id: "abc".into(),
        trigger: "帮我写测试".into(),
        content: "先列要点。".into(),
    }];
    assert!(take_match(&hits, "帮我写测试").is_some(), "逐字相同 ⇒ 命中");
    assert!(
        take_match(&hits, "  帮我写测试\n").is_some(),
        "首尾空白由 trim 归一（与写方同一对函数）"
    );
    assert!(
        take_match(&hits, "帮我写测试吧").is_none(),
        "更宽的『同类』判定不在本批：恒等就是恒等"
    );
    assert!(take_match(&hits, "").is_none(), "空发言不命中");
    assert!(
        take_match(&[], "帮我写测试").is_none(),
        "没有可用技能 ⇒ 恒不命中"
    );

    // 截断同源：超长发言按同一个上限截断后，与写方存下的 trigger 对得上。
    let max = super::super::v2_memory::ENCODE_MAX_CHARS;
    let long = "甲".repeat(max + 50);
    let hits = vec![SkillLlmHit {
        skill_id: "abc".into(),
        trigger: super::super::v2_memory::truncate_chars(&long, max),
        content: "先列要点。".into(),
    }];
    assert!(
        take_match(&hits, &long).is_some(),
        "长发言按同一个上限截断后命中（读方另写一份截断逻辑就会永远命不中）"
    );
}

/// 链路前提（[plan/12 批 1](../../../../docs/plan/12-价值验收与基线埋点.md) 的「写侧
/// 前提」+ [plan/11 批 2 ③](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
/// 的接线判据）：技能入格后，**生产读方** `v2_memory::recall_view` 必须把它召回，
/// 随后 `route` 必须真的判出观测。
///
/// 反向即断链：`memory × *` 的 actor 是属主（`PRINCIPAL_USER`），写方归位前是 core
/// 的占位 `"agent:main"`，召回投影按 `actor == viewer`（viewer = 属主）过滤 ⇒ 技能被
/// 挡在视图外 ⇒ `route` 恒快判返回、观测永不落格、校准账零使用——单测用错了 viewer
/// （`agent:main`）时这条链**看起来**是通的，只有走生产读方才看得见。
#[test]
fn the_production_recall_view_sees_the_compiled_skill() {
    // `recall_view` 扫 `<根>/*​/v2-events.wal`：根自己造，不借全局临时目录
    //（否则会扫进并行跑的其它用例的会话）。
    let root = tempfile::tempdir().expect("临时根");
    let session = root.path().join("s-1");
    std::fs::create_dir_all(&session).expect("会话目录");
    {
        let store = EventWalStore::open(wal_of(&session)).expect("WAL");
        let anchor = seed_turn(&store);
        let snap = store.range(Seq::new(0));
        compile(
            &store,
            &snap,
            0,
            anchor,
            "帮我写测试",
            "好的，先写正向用例。",
            1_700_000_000_000,
        )
        .expect("编译入格")
        .expect("首次必须产出技能");
    }

    let mut view =
        super::super::v2_memory::recall_view(&session).expect("技能是一条记忆 ⇒ 视图非空");
    assert!(
        view.entries.iter().any(|e| e.tag == SKILL_TAG),
        "生产读方必须召回技能：{:?}",
        view.entries
    );

    // 路由半边真的判得动：生产视图 → 观测（零使用 ⇒ 快路、不摘条目）。
    let routing = route(&session, &mut view);
    assert_eq!(
        routing.obs.len(),
        1,
        "生产视图 → 路由必须产出一条观测（断链时这里是 0）：{:?}",
        routing.obs
    );
    assert!(
        !routing.obs[0].1,
        "零使用的新技能不判死 ⇒ 快路 fallback = false"
    );
    assert_eq!(
        routing.hits.len(),
        1,
        "同一次判定还要产出可用集（执行侧靠它装配反射档）"
    );
}
