//! 技能编译与路由验收（S9 步 22，[04 §3.1 批⑪](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
//! **编译 → 观测 → 校准 → 回退**的每一环都要看得见，且每条口径各配一个反向用例
//! （同 trigger 不重编译；零使用不判死、一次回退即摘；本轮没技能 ⇒ 连投影都不跑）。

use super::{compile, route, take_match, SkillCompiler, SkillLlmHit, SkillRoute, SkillRouter};
use crate::control_decl::guard_vocab;
use symbio_core::SKILL_TAG;
use symbio_core::{
    calibration, check_all, Budget, Entity, Event, EventStore, Seq, Store, Verb,
    EVENT_ASSISTANT_FINAL, EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED,
    EVENT_SESSION_CONSOLIDATED, EVENT_USER_MESSAGE,
};
use symbio_core::{PRINCIPAL_MAIN, PRINCIPAL_USER};

use super::super::recall::recall;

/// 一轮的**溯源锚**：`user.message` 开轮 + `assistant.final` 收束——
/// 技能的 `produced_by` 指向开轮格（I2），`check_all` 才看得见整条链是绿的。
fn seed_turn(store: &EventStore) -> u64 {
    let anchor = store
        .append(
            Event::pending(
                "v2u-test-a0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                symbio_core::PRINCIPAL_USER,
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

/// 一条路由观测（生产写方是 `derived_facts::record_learning`，这里直写**同一形状**——
/// `calibration` 认**自己的格子** `memory.calibrated`，并逐个校载荷字段）。
fn observe(store: &EventStore, n: usize, skill_id: &str, fallback: bool, anchor: u64) {
    store
        .append(
            Event::pending(
                format!("v2s-test-{n}"),
                EVENT_MEMORY_CALIBRATED,
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
/// viewer 取**属主**（与生产读方 `memory_events::recall_view` 同一个 viewer）：
/// `memory × *` 的 actor 是属主、投影按 `actor == viewer` 过滤，两个 viewer
/// 不一致时单测看得到的东西生产看不到（断链就是这么溜进来的）。
fn view_of(store: &EventStore) -> super::super::recall::RecallView {
    recall(PRINCIPAL_USER, None, None)
        .apply(&store.range(Seq::new(0)), i64::MAX, Budget::generous())
        .value
}

/// 写一轮：编译出一条技能，返回它的 `skill_id`（锚 = 开轮格）。
fn seed_skill(store: &EventStore, trigger: &str) -> String {
    let anchor = seed_turn(store);
    let snap = store.range(Seq::new(0));
    let seq = compile(
        store,
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
    let store = EventStore::new();
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
        skill.payload.get("vis_scope").and_then(|v| v.as_str()),
        Some("shared"),
        "技能 = 固化的可复用经验 ⇒ shared（T-117：缺席 = 缺省私有，\
         跨会话命中——t35 证据②——就被召回预过滤挡死）"
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
    assert!(
        check_all(guard_vocab(), &snap).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &snap)
    );

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
    let store = EventStore::new();
    let skill_id = seed_skill(&store, "帮我写测试");

    let mut view = view_of(&store);
    assert_eq!(
        view.entries.len(),
        1,
        "只召回技能本身（观测是 memory.recalled，不进召回投影）"
    );
    assert_eq!(view.entries[0].tag, SKILL_TAG);

    let routing = route(&store.range(Seq::new(0)), &mut view);
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
    let store = EventStore::new();
    let skill_id = seed_skill(&store, "帮我写测试");
    {
        // 观测的溯源锚沿用开轮格：从快照里取第一条 `user.message` 的 seq。
        let anchor = store
            .range(Seq::new(0))
            .iter()
            .find(|e| e.kind == EVENT_USER_MESSAGE)
            .and_then(|e| e.seq)
            .expect("开轮格");
        observe(&store, 0, &skill_id, true, anchor.value());
    }

    let mut view = view_of(&store);
    assert_eq!(view.entries.len(), 1, "技能本来在视图里");

    let routing = route(&store.range(Seq::new(0)), &mut view);
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

/// **快照边界不缩小校准账**（P6-② 的愈合点，[plan/14 §2.2](../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 推论 1）：
/// 技能与它的回退观测都写在快照之前 ⇒ 压缩之后这条技能**仍**判不可信、仍被摘出
/// 本轮视图。判据问的是「这条技能被记过回退没有」，与那段历史之后被压掉没有无关。
/// 读方吃的是事实源的**完整史**（`Store::range`，D-085），不是「从最新快照起的
/// 现行窗口」；归档本身的存储语义由 session 存储层（`store/wal.test.rs`）钉住。
///
/// **反向自检**：把 `route` 里的 `store.range(Seq::new(0))` 换成「只切最新快照之后
/// 的窗口」⇒ 本用例红（观测看不见 ⇒ 判成快路，被摘过的技能重新进提示词——静默的复发）。
#[test]
fn an_earlier_fallback_still_demotes_the_skill_across_a_snapshot() {
    let store = EventStore::new();
    let skill_id = seed_skill(&store, "帮我写测试");
    let anchor = store
        .range(Seq::new(0))
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.seq)
        .expect("开轮格");
    observe(&store, 0, &skill_id, true, anchor.value());

    // 压缩在技能与观测之后发生：快照格之后，上面两格都属于「被取代的前缀」。
    store
        .append(
            Event::pending(
                "v2sc-test-snapshot-boundary",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                0,
                PRINCIPAL_MAIN,
            )
            .with_payload(serde_json::json!({ "messages": [], "turns": 1 })),
        )
        .expect("快照入格");

    let mut view = view_of(&store);
    assert_eq!(view.entries.len(), 1, "压缩前编译的技能仍召回得到");
    let routing = route(&store.range(Seq::new(0)), &mut view);
    assert_eq!(
        routing.obs,
        vec![(skill_id, true)],
        "快照前缀里那条回退观测照样作数"
    );
    assert!(view.entries.is_empty(), "判过回退 ⇒ 摘出本轮视图");
}

/// 反向（读侧的平凡值）：本轮没召回技能 ⇒ 直接返回，不跑任何投影
/// ——默认关掉编译时读侧恒走这一支，不成为每轮的固定开销。
#[test]
fn without_any_skill_the_reader_touches_nothing() {
    let store = EventStore::new();
    let _ = seed_turn(&store);
    let mut view = view_of(&store);
    assert!(view.entries.is_empty(), "没有技能可召回");

    let routing = route(&store.range(Seq::new(0)), &mut view);
    assert!(routing.obs.is_empty() && routing.hits.is_empty());
    assert!(view.entries.is_empty());
    // 空快照同样不炸（读方不取事实、不创建文件）。
    let routing = route(&[], &mut view);
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
    let max = super::super::memory_events::ENCODE_MAX_CHARS;
    let long = "甲".repeat(max + 50);
    let hits = vec![SkillLlmHit {
        skill_id: "abc".into(),
        trigger: symbio_core::merge_kernel::truncate_chars(&long, max),
        content: "先列要点。".into(),
    }];
    assert!(
        take_match(&hits, &long).is_some(),
        "长发言按同一个上限截断后命中（读方另写一份截断逻辑就会永远命不中）"
    );
}

/// 链路前提（[plan/12 批 1](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的「写侧
/// 前提」+ plan/11 批 2 ③
/// 的接线判据）：技能入格后，**生产读方** `memory_events::recall_view` 必须把它召回，
/// 随后 `route` 必须真的判出观测。
///
/// 反向即断链：`memory × *` 的 actor 是属主（`PRINCIPAL_USER`），写方归位前是 core
/// 的占位 `"session/main"`，召回投影按 `actor == viewer`（viewer = 属主）过滤 ⇒ 技能被
/// 挡在视图外 ⇒ `route` 恒快判返回、观测永不落格、校准账零使用——单测用错了 viewer
/// （`agent:main`）时这条链**看起来**是通的，只有走生产读方才看得见。
#[test]
fn the_production_recall_view_sees_the_compiled_skill() {
    let store = EventStore::new();
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

    // 存储层交值：本会话自己的事实切片（`own = true`）。
    let facts = store.range(Seq::new(0));
    let mut view = super::super::memory_events::recall_view(
        vec![symbio_core::FleetSnapshot {
            own: true,
            events: facts.clone(),
        }],
        None,
        "session/main",
    )
    .expect("技能是一条记忆 ⇒ 视图非空");
    assert!(
        view.entries.iter().any(|e| e.tag == SKILL_TAG),
        "生产读方必须召回技能：{:?}",
        view.entries
    );

    // 路由半边真的判得动：生产视图 → 观测（零使用 ⇒ 快路、不摘条目）。
    let routing = route(&facts, &mut view);
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

// ── 编译与路由单元验收（D-189：实现随唯一消费方住本插件） ──

/// S11 §6 验收 1 + 4：编译产出的技能事件 produced_by 必须非空；置空 → I2 抓住。
#[test]
fn compiled_skill_requires_provenance() {
    let compiler = SkillCompiler;
    // 源轨迹：一个成功任务的终态（seq 2）。
    let skill = compiler.compile("weekly-report", "用户问周报格式", "按模板三段式作答", 2);
    assert_eq!(skill.produced_by, Some(2), "溯源 100%：指向源轨迹");
    assert_eq!(skill.payload["tag"], "skill");
    // 正向：I2 全绿。
    assert!(symbio_core::provenance_violation(&skill).is_none());
    // 验收 4（反向）：把 produced_by 置空 → 断言 1 必须失败。
    let bare = Event {
        produced_by: None,
        ..skill
    };
    let bad = symbio_core::provenance_violation(&bare).is_some();
    assert!(bad, "无溯源的技能必须被看见");
}

/// S11 §6 验收 2 + 3：命中技能 cost 显著低于完整推理；置信度低于阈值必回退。
#[test]
fn skill_hit_is_cheaper_and_low_confidence_falls_back() {
    let store = EventStore::new();
    let src = seed_turn(&store);
    let compiler = SkillCompiler;
    // 编译技能（源轨迹 = 那一轮的开轮格）。
    store
        .append(compiler.compile("weekly-report", "周报", "三段式", src))
        .unwrap();
    // 命中：反射档快路（cost 40ms，低两个数量级）。
    store
        .append(
            Event::pending(
                "h1",
                EVENT_MEMORY_CALIBRATED,
                Entity::Memory,
                Verb::Asserted,
                0,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(src)
            .with_cost_ms(40)
            .with_payload(serde_json::json!({ "skill_id": "weekly-report", "fallback": false })),
        )
        .unwrap();
    // 未命中 / 回退：完整推理（cost 5000ms）。
    store
        .append(
            Event::pending(
                "h2",
                EVENT_MEMORY_CALIBRATED,
                Entity::Memory,
                Verb::Asserted,
                0,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(src)
            .with_cost_ms(5_000)
            .with_payload(serde_json::json!({ "skill_id": "weekly-report", "fallback": true })),
        )
        .unwrap();
    let snapshot = store.range(Seq::new(0));

    // 验收 2：命中路径 cost 显著低于回退路径（40ms vs 5000ms）。
    let hit_cost = snapshot
        .iter()
        .find(|e| {
            e.kind == EVENT_MEMORY_CALIBRATED
                && e.payload.get("fallback") == Some(&serde_json::json!(false))
        })
        .map(|e| e.cost_ms)
        .unwrap();
    let fallback_cost = snapshot
        .iter()
        .find(|e| {
            e.kind == EVENT_MEMORY_CALIBRATED
                && e.payload.get("fallback") == Some(&serde_json::json!(true))
        })
        .map(|e| e.cost_ms)
        .unwrap();
    assert!(
        hit_cost * 10 < fallback_cost,
        "技能命中必须显著更便宜：{hit_cost} vs {fallback_cost}"
    );

    // 校准：1 用 1 回退 ⇒ 置信度 0.5；低于阈值 0.8 ⇒ 必须回退（不得走技能路径）。
    let cal = calibration().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(cal.value.of("weekly-report").uses, 2);
    assert_eq!(cal.value.of("weekly-report").fallbacks, 1);
    let router = SkillRouter;
    assert_eq!(
        router.route(cal.value.of("weekly-report").confidence(), 0.8),
        SkillRoute::ReasonerFallback { budget_ms: 300 },
        "校准值持续走低 ⇒ 保留反自动化回退"
    );
    // 反向：置信度高于阈值 → 走快路。
    assert_eq!(
        router.route(0.95, 0.8),
        SkillRoute::SkillFastPath { budget_ms: 80 }
    );
    // N1：校准双跑一致。
    assert_eq!(
        cal.value,
        calibration()
            .apply(&snapshot, 9_999, Budget::generous())
            .value
    );
    // 全链路不变量仍绿。
    assert!(
        check_all(guard_vocab(), &snapshot).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &snapshot)
    );
}
