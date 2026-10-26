//! 自主触发落格的单元测试。
//!
//! 与实现**同级**分文件（`autonomous.rs` + `#[path]` 指定的本文件，见 `CONTRIBUTING.md`）。

use super::*;
use crate::plugin::SessionConfig;
use symbio_core::{check_all, Entity, Event, Verb, EVENT_USER_MESSAGE};
use std::sync::Arc;

// 这一段钉的不是"触发器会不会开跑一轮"（那要真模型，归 e2e），而是**触发先落事实**
// 这条链子：`system.triggered` 必须存在且溯源指向更早的格子、健康自检紧随其后、
// 「欲」永远入格、任务却**只在闸门放行时**才长出来。分开断言是因为三者各自的失效
// 都不报错：漏了触发 = 自主行为查无此事；漏了自检 = `system` 实体只开不进；
// 漏了闸门 = 自己写自己的对话（[roadmap/S12 §5](../../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")「自主写入对话 0」）。

fn autonomy_plugin(conation_enabled: bool) -> (tempfile::TempDir, Arc<SessionPlugin>) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = Arc::new(SessionPlugin::new(
        None,
        SessionConfig {
            conation_enabled,
            ..SessionConfig::default()
        },
        symbio_core::PluginDir::at(dir.path(), "session"),
    ));
    (dir, plugin)
}

fn wal(p: &SessionPlugin, sid: &str) -> std::path::PathBuf {
    crate::paths::session_dir(&p.storage_dir(), sid)
        .join(crate::paths::V2_WAL_FILE)
}

/// 事实源里先放一格：溯源锚指"上一格"，一格都没有就没有可触发的东西。
fn seed_one_fact(p: &SessionPlugin, sid: &str) {
    let path = wal(p, sid);
    std::fs::create_dir_all(path.parent().expect("WAL 路径必有父目录")).expect("会话目录创建失败");
    EventWalStore::open(&path)
        .expect("打开 WAL")
        .append(Event::pending(
            "u1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "user:t",
        ))
        .expect("事实入格");
}

fn read_wal(p: &SessionPlugin, sid: &str) -> Vec<Event> {
    EventWalStore::open_readonly(wal(p, sid))
        .expect("只读打开 WAL")
        .range(Seq::new(0))
}

/// 主链路：触发 → 自检 → 欲三格按序入格、溯源逐级指向更早的格子；
/// 闸门的平凡值 `enabled = false` ⇒ 一条自主任务都不产生（E3）。
#[tokio::test]
async fn trigger_facts_land_in_order_and_the_gate_stays_closed_by_default() {
    let (_dir, p) = autonomy_plugin(false);
    let sid = "s-hb";
    seed_one_fact(&p, sid);

    assert!(
        p.record_autonomous_trigger(sid, 9_999, "检查一下待办")
            .await,
        "事实源在、没有写方在写 ⇒ 这一趟该触发"
    );

    let events = read_wal(&p, sid);
    assert_eq!(
        events.len(),
        4,
        "种子 1 格 + 触发 / 自检 / 欲 3 格；闸门关着不该有第 5 格"
    );
    let at = |e: Entity, v: Verb| {
        events
            .iter()
            .position(|x| x.entity == e && x.verb == v)
            .expect("该有的那一格")
    };
    let triggered = at(Entity::System, Verb::Opened);
    let health = at(Entity::System, Verb::Progressed);
    let want = at(Entity::Conation, Verb::Opened);
    assert_eq!(
        (triggered, health, want),
        (1, 2, 3),
        "触发 → 自检 → 欲 的顺序"
    );
    assert!(
        !events
            .iter()
            .any(|e| e.entity == Entity::Task && e.verb == Verb::Opened),
        "ConationPolicy 平凡值 false ⇒ 不升格为任务（E3）"
    );

    // 溯源必须指向**更早**的格子：触发接种子、自检与欲接触发。
    let seq = |i: usize| events[i].seq.expect("已入格必有 seq").value();
    assert_eq!(events[triggered].produced_by, Some(0), "锚 = 上一格");
    assert_eq!(events[health].produced_by, Some(seq(triggered)));
    assert_eq!(events[want].produced_by, Some(seq(triggered)));
    assert!(
        seq(triggered) < seq(health) && seq(health) < seq(want),
        "溯源方向不许指自己或更晚的格子"
    );
    // 自检修的是**这次触发**测得的空闲时长——`system.health` 的载荷是数据不是装饰。
    assert_eq!(
        events[health].payload.get("idle_ms"),
        Some(&serde_json::json!(9_999))
    );

    assert!(check_all(&events).is_empty(), "{:?}", check_all(&events));
}

/// 闸门打开后长目标**只开一次**：自主触发按间隔周期性表达同一条欲，
/// 每次都开格会让就绪集无界增长——而 readyset 是模型唯一的调度候选来源，那就等于
/// 用自己刷爆自己的提示词。而「欲」是流不是状态，照样每次入格。
#[tokio::test]
async fn gate_opens_the_long_goal_exactly_once_per_goal() {
    let (_dir, p) = autonomy_plugin(true);
    let sid = "s-gate";
    seed_one_fact(&p, sid);

    assert!(p.record_autonomous_trigger(sid, 60_000, "把周报写了").await);
    assert!(p.record_autonomous_trigger(sid, 60_000, "把周报写了").await);

    let events = read_wal(&p, sid);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.entity == Entity::Task && e.verb == Verb::Opened)
            .count(),
        1,
        "同一条长目标只声明一次"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.entity == Entity::Conation && e.verb == Verb::Opened)
            .count(),
        2,
        "「欲」每次触发照样入格（它是流，不是状态）"
    );
    assert!(check_all(&events).is_empty(), "{:?}", check_all(&events));
}

/// 两种不触发各有各的理由，而且**都不动文件**：
/// 1. 事实源不存在 ⇒ 没有可指的溯源锚（I2：宁可不触发，也不写一条来路不明的触发）；
/// 2. 有写方在写 ⇒ 事实源是同一会话的唯一写入口，两个写方并存会重复 `head`。
#[tokio::test]
async fn no_fact_source_or_a_live_writer_both_block_the_trigger() {
    let (_dir, p) = autonomy_plugin(false);
    let sid = "s-none";

    assert!(
        !p.record_autonomous_trigger(sid, 100, "看一眼").await,
        "一格事实都没有 ⇒ 无处可溯，不触发"
    );
    assert!(!wal(&p, sid).exists(), "不触发就一格都不写（连文件都不建）");

    seed_one_fact(&p, sid);
    let state = p.active_mgr.get_or_create(sid).await;

    // 收束期写方（v2 运行器的轮末收束）跑在 `is_working` 复位之前 ⇒ 忙窗里让开。
    state.inner.write().await.set_working_for_test(true);
    assert!(
        !p.record_autonomous_trigger(sid, 100, "看一眼").await,
        "收束期写方在跑 ⇒ 让开"
    );
    assert_eq!(read_wal(&p, sid).len(), 1, "忙时一格都不写");

    // 收件箱的空闲写方只在 `preempt != None` 时才落格 ⇒ 未结清时让开。
    state.inner.write().await.set_working_for_test(false);
    state.inner.write().await.preempt = PreemptPending::Resume {
        task_id: "t1".to_string(),
    };
    assert!(
        !p.record_autonomous_trigger(sid, 100, "看一眼").await,
        "抢占待结算 ⇒ 让开"
    );
    assert_eq!(read_wal(&p, sid).len(), 1, "两个空闲写方不许同时开档");
}

/// 闸门的第二条规则也在生产里：目标过宽 ⇒ 拒（`conation_enabled = true` 也拦得住）。
///
/// 拒绝同样是**事实**——触发与欲照样入格，只是不长成任务；静默拒绝就等于没有闸门。
#[tokio::test]
async fn an_over_broad_goal_is_refused_even_with_the_gate_open() {
    let (_dir, p) = autonomy_plugin(true);
    let sid = "s-broad";
    seed_one_fact(&p, sid);

    let broad = "x".repeat(201); // > ConationPolicy::default().max_goal_len（200）
    assert!(p.record_autonomous_trigger(sid, 1, &broad).await);

    let events = read_wal(&p, sid);
    assert!(
        !events
            .iter()
            .any(|e| e.entity == Entity::Task && e.verb == Verb::Opened),
        "过宽的目标不升格"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.entity == Entity::Conation && e.verb == Verb::Opened)
            .count(),
        1,
        "欲照样入格——拒绝是可查的结论，不是静默"
    );
    assert!(check_all(&events).is_empty(), "{:?}", check_all(&events));
}

// ==================== 发起者与意欲闸门的单元验收（D-189） ====================

use symbio_core::{
    budget_exceeded, EventStore, EVENT_CONATION_EXPRESSED, EVENT_SYSTEM_TRIGGERED,
    EVENT_TASK_PROGRESS,
};
use symbio_core::{PRINCIPAL_AUTONOMOUS, PRINCIPAL_USER, PRINCIPAL_WORKER};

/// 事实源里先放一格：触发是"从当时的状态出发"的，必须先有可指的锚。
fn seed_root(store: &EventStore) -> u64 {
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            PRINCIPAL_USER,
        ))
        .expect("根事件入格")
        .value()
}

/// S12 §6 验收 1：无用户消息时，定时触发必须产生 `system.triggered` 事件
/// （触发器产出事件，不是旁路——自主行为同样走 I1 单通道 + I2 溯源）。
#[test]
fn scheduled_trigger_produces_event_not_side_channel() {
    let store = EventStore::new();
    // 本轮**没有新的用户消息**——纯自主场景；但触发必须指得着一条更早的事实：
    // 生产路径在 `head == 0` 时直接放弃触发（`transcript/autonomous.rs`：
    // "还没有事实源，本次不触发（无可指的溯源锚）"）。夹具照生产给一个锚。
    let src = seed_root(&store);
    let init = AutonomousInitiator;
    store.append(init.trigger(src)).unwrap();
    let snapshot = store.range(Seq::new(0));
    assert!(
        snapshot.iter().any(|e| e.kind == EVENT_SYSTEM_TRIGGERED),
        "自主行为必须以事件形态存在"
    );
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// S12 §6 验收 3 + 4：长目标超过自主层预算 → 必须被看见；预算改小 → 判定改变。
#[test]
fn long_goal_overrun_is_visible_and_budget_param_is_live() {
    let store = EventStore::new();
    let src = seed_root(&store);
    let init = AutonomousInitiator;
    store.append(init.trigger(src)).unwrap();
    store
        .append(init.open_long_goal("goal-1", "整理全年归档", src))
        .unwrap();
    // 任务执行事件：耗时 90M ms > 自主层预算 86.4M。
    store
        .append(
            Event::pending(
                "g1",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                PRINCIPAL_AUTONOMOUS,
            )
            .with_produced_by(src)
            .with_cost_ms(90_000_000)
            .with_payload(serde_json::json!({ "task_id": "goal-1" })),
        )
        .unwrap();
    let snapshot = store.range(Seq::new(0));
    // 验收 3：超 86.4M 必须产出兜底（先被看见）。
    let bad = budget_exceeded(
        &snapshot,
        Some(symbio_core::LatencyTier::Autonomic.budget_ms()),
    );
    assert_eq!(bad.len(), 1, "长目标超支必须被看见：{bad:?}");
    // 验收 4（反向）：同一事件 70M ms（预算内）；把预算改成 60000 → 判定改变。
    let ok_event = Event::pending(
        "g2",
        EVENT_TASK_PROGRESS,
        Entity::Task,
        Verb::Progressed,
        0,
        PRINCIPAL_AUTONOMOUS,
    )
    .with_produced_by(src)
    .with_cost_ms(70_000)
    .with_payload(serde_json::json!({ "task_id": "goal-1" }));
    let ok_events = vec![ok_event];
    assert!(
        budget_exceeded(&ok_events, Some(86_400_000)).is_empty(),
        "70M 在自主层预算内"
    );
    assert_eq!(
        budget_exceeded(&ok_events, Some(60_000)).len(),
        1,
        "同一耗时，深挖档预算下违规"
    );
}

/// 02 §2.3 欲治理三条：(a) 候选造出来一定未批准 (b) 无溯源构造不出 (c) 关停后
/// 无任何自主任务但读侧投影照常（J2 平凡值）。ZST 令牌零开销。
#[test]
fn conation_governance_three_rules() {
    let init = AutonomousInitiator;
    let store = EventStore::new();
    let src = seed_root(&store);
    store.append(init.trigger(src)).unwrap();
    store.append(init.express_intent("整理归档", src)).unwrap();
    let snapshot = store.range(Seq::new(0));

    // (a) 唯一构造路径读出的候选，approved 恒 false——欲不得直接变成行。
    let want = snapshot
        .iter()
        .find(|e| e.kind == EVENT_CONATION_EXPRESSED)
        .unwrap();
    let mut candidate = ConationCandidate::from_event(want).expect("合法欲事件必出候选");
    assert!(!candidate.is_approved());
    // approved 字段私有：外部无法伪造 true（error[E0451]，编译期反向用例）。

    // (b) 无溯源的欲 → 构造不出候选（I2 强化）。
    let bare = Event {
        produced_by: None,
        ..Event::pending(
            "w1",
            EVENT_CONATION_EXPRESSED,
            Entity::Conation,
            Verb::Opened,
            0,
            PRINCIPAL_USER,
        )
    };
    assert!(
        ConationCandidate::from_event(&bare).is_none(),
        "来路不明的意图混不进来"
    );

    // (c) 关停开关：enabled=false → 评估拒绝，无自主任务；但读侧照常工作。
    let policy_off = ConationPolicy::default();
    let grants = symbio_core::production_matrix();
    assert_eq!(
        IntentGate::evaluate(&candidate, &policy_off, grants, PRINCIPAL_AUTONOMOUS),
        IntentDecision::Rejected("conation disabled")
    );
    // 打开后：评估通过 → 升格；溯源锚与目标都从 `ApprovedIntent` 读（字段私有）。
    let policy_on = ConationPolicy {
        enabled: true,
        max_goal_len: 200,
    };
    let approved = IntentGate::approve(&mut candidate, &policy_on, grants, PRINCIPAL_AUTONOMOUS)
        .expect("合法意图必批");
    assert!(candidate.is_approved());
    assert_eq!(approved.goal(), "整理归档");
    assert_eq!(
        approved.source_seq(),
        want.seq.map(|s| s.value()).unwrap_or(0)
    );

    // (d) 授权是真闸：开关打开，但不持 `define.work` 的主体升格被拒。
    let mut other = ConationCandidate::from_event(want).expect("同一事件可再造候选");
    assert_eq!(
        IntentGate::approve(&mut other, &policy_on, grants, PRINCIPAL_WORKER),
        Err("unauthorized: define.work"),
        "worker 不持 define.work"
    );
    assert!(!other.is_approved(), "被拒的候选不得变成已批准");
}
