//! 自主触发落格的单元测试。
//!
//! 与实现**同级**分文件（`autonomous.rs` + `#[path]` 指定的本文件，见 `CONTRIBUTING.md`）。

use super::*;
use crate::plugin::SessionConfig;
use std::sync::Arc;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_plugin_api::events::{EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE};
use symbio_plugin_api::vocab;
use symbio_sdk::invariants::check_all;

// 这一段钉的不是"触发器会不会开跑一轮"（那要真模型，归 e2e），而是**触发先落事实**
// 这条链子：`system.triggered` 必须存在且溯源指向更早的格子、健康自检紧随其后、
// 「欲」永远入格。三者各自的失效都不报错：漏了触发 = 自主行为查无此事；漏了自检 =
// `system` 实体只开不进；漏了「欲」= 这条触发没有留下可升格的数据
// （[DE-042 §5](../../../../docs/003-design/042-自主层与长期目标.md "DE-042-自主层与长期目标")「自主写入对话 0」）。
//
// 升格不在这里：它是 conation 自申报的债 + 执行体（`D-386`）。本二进制不链接那个包
// （E-009），所以这里只能钉「读不到 ⇒ 不升格」；「只开一次」的组合证明在
// `e2e/cases/t36-autonomous-initiator.mjs`。

fn autonomy_plugin(conation_enabled: bool) -> (tempfile::TempDir, Arc<SessionPlugin>) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = Arc::new(SessionPlugin::new(
        None,
        SessionConfig {
            conation_enabled,
            ..SessionConfig::default()
        },
        symbio_sdk::plugin::PluginDir::at(dir.path(), "session"),
    ));
    (dir, plugin)
}

fn wal(p: &SessionPlugin, sid: &str) -> std::path::PathBuf {
    crate::paths::session_dir(&p.storage_dir(), sid).join(crate::paths::V2_WAL_FILE)
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
            Entity::Scope,
            Verb::Opened,
            "user:t",
        ))
        .expect("事实入格");
}

fn read_wal(p: &SessionPlugin, sid: &str) -> Vec<Event> {
    EventWalStore::open_readonly(wal(p, sid))
        .expect("只读打开 WAL")
        .range(Seq::new(0))
}

/// 轮首装配那一步的等价物：把开关按共享名写进 `session.config` 快照。
///
/// 闸门读的是这条事实（`D-252` / `D-379`），不是插件里的活配置——因此凡是
/// 「闸门应当开着」的用例都必须先有这一格。
fn seed_config(p: &SessionPlugin, sid: &str, payload: serde_json::Value) {
    EventWalStore::open(wal(p, sid))
        .expect("打开 WAL")
        .append(
            Event::pending(
                "u2",
                EVENT_SESSION_CONFIG,
                Entity::Scope,
                Verb::Asserted,
                "user:t",
            )
            .with_produced_by(0)
            .with_payload(payload),
        )
        .expect("配置快照入格");
}

/// 主链路：触发 → 自检 → 欲三格按序入格、溯源逐级指向更早的格子；
/// 升格由能力包自己的债与执行体做，本二进制不链接它 ⇒ 一条自主任务都不产生（T3）。
#[tokio::test]
async fn trigger_facts_land_in_order_and_no_task_grows_without_the_capability_pack() {
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
        "能力包不在 ⇒ 读不到判决 ⇒ 不升格为任务（T3）"
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

    assert!(
        check_all(vocab(), &events).is_empty(),
        "{:?}",
        check_all(vocab(), &events)
    );
}

/// **能力包不在 ⇒ 升格这一步消失**（T3 形状）：触发与欲照样入格，但不长出任务。
///
/// ## 为什么这里断言的是「不开」而不是「只开一次」
///
/// 升格判据（开关 / 授权 / 已声明）**归能力包**：`crates/symbio-plugin-conation` 的
/// `DebtSource::rows` 算「哪些目标算已声明」（`D-386`）。本二进制不链接那个包（E-009），
/// 所以这里读到的必然是**可诊断的缺席**——本文件因此只能钉本插件剩下的那一半：
/// **读不到 ⇒ 不升格**（而不是「读不到 ⇒ 当作没声明过，于是每 tick 重开一条」，
/// 那会拿重复开工掩盖一个装配问题）。
#[tokio::test]
async fn without_the_capability_pack_nothing_promotes() {
    let (_dir, p) = autonomy_plugin(true);
    let sid = "s-gate";
    seed_one_fact(&p, sid);
    // 开关按事实打开：能力缺席是这条用例唯一的变量。
    seed_config(&p, sid, serde_json::json!({ "conation_enabled": true }));

    // 触发照常发生（返回 true = 这一轮照常开跑）：能力缺席不该连累开轮。
    assert!(p.record_autonomous_trigger(sid, 60_000, "把周报写了").await);
    assert!(p.record_autonomous_trigger(sid, 60_000, "把周报写了").await);

    let events = read_wal(&p, sid);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.entity == Entity::Task && e.verb == Verb::Opened)
            .count(),
        0,
        "读不到「已声明哪些目标」⇒ 不升格（宁可这一次不开，也不每 tick 重开一条）"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.entity == Entity::Conation && e.verb == Verb::Opened)
            .count(),
        2,
        "「欲」每次触发照样入格（它是流，不是状态；能力缺席不影响表达）"
    );
    assert!(
        check_all(vocab(), &events).is_empty(),
        "{:?}",
        check_all(vocab(), &events)
    );
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

// ==================== 发起者与预算的单元验收（D-189） ====================
use crate::declarations::EVENT_SYSTEM_TRIGGERED;
use crate::store::EventWalStore;
use symbio_kernel::store::EventStore;
use symbio_kernel::Seq;
use symbio_plugin_api::events::{EVENT_TASK_OPENED, EVENT_TASK_PROGRESS};
use symbio_sdk::authz::PRINCIPAL_AUTONOMOUS;
use symbio_sdk::authz::PRINCIPAL_USER;
use symbio_sdk::invariants::budget_exceeded;

/// 事实源里先放一格：触发是"从当时的状态出发"的，必须先有可指的锚。
fn seed_root(store: &EventStore) -> u64 {
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Scope,
            Verb::Opened,
            PRINCIPAL_USER,
        ))
        .expect("根事件入格")
        .value()
}

/// DE-042 §6 验收 1：无用户消息时，定时触发必须产生 `system.triggered` 事件
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
        check_all(vocab(), &snapshot).is_empty(),
        "{:?}",
        check_all(vocab(), &snapshot)
    );
}

/// DE-042 §6 验收 3 + 4：长目标超过自主层预算 → 必须被看见；预算改小 → 判定改变。
///
/// 升格本身归 conation 的债与执行体（`D-386`）；这里只钉预算读数对**升格产物形状**
/// 的反应——`task.opened` 的 `budget_ms` 就是自主层的 86.4M，没超过时读数不该报。
#[test]
fn long_goal_overrun_is_visible_and_budget_param_is_live() {
    let store = EventStore::new();
    let src = seed_root(&store);
    let init = AutonomousInitiator;
    store.append(init.trigger(src)).unwrap();
    store
        .append(
            Event::pending(
                "o-hb-1",
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                PRINCIPAL_AUTONOMOUS,
            )
            .with_produced_by(src)
            .with_payload(serde_json::json!({
                "task_id": "hb-1",
                "depends_on": [],
                "goal": "整理全年归档",
                "budget_ms": symbio_kernel::adapters::LatencyTier::Autonomic.budget_ms(),
            })),
        )
        .unwrap();
    // 任务执行事件：耗时 90M ms > 自主层预算 86.4M。
    store
        .append(
            Event::pending(
                "g1",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                PRINCIPAL_AUTONOMOUS,
            )
            .with_produced_by(src)
            .with_cost_ms(90_000_000)
            .with_payload(serde_json::json!({ "task_id": "hb-1" })),
        )
        .unwrap();
    let snapshot = store.range(Seq::new(0));
    // 验收 3：超 86.4M 必须产出兜底（先被看见）。
    let bad = budget_exceeded(
        &snapshot,
        Some(symbio_kernel::adapters::LatencyTier::Autonomic.budget_ms()),
    );
    assert_eq!(bad.len(), 1, "长目标超支必须被看见：{bad:?}");
    // 验收 4（反向）：同一事件 70M ms（预算内）；把预算改成 60000 → 判定改变。
    let ok_event = Event::pending(
        "g2",
        EVENT_TASK_PROGRESS,
        Entity::Task,
        Verb::Progressed,
        PRINCIPAL_AUTONOMOUS,
    )
    .with_produced_by(src)
    .with_cost_ms(70_000)
    .with_payload(serde_json::json!({ "task_id": "hb-1" }));
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
