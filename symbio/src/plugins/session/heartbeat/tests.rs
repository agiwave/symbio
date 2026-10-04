//! `heartbeat` 模块的单元测试。
//!
//! 与实现**同级**分文件（`heartbeat/mod.rs` + 同级 `tests.rs`，见 `CONTRIBUTING.md`）：
//! `mod.rs` 只保留生产代码，测试全部放本文件。

use super::*;

/// 空闲基线取较新者：正常路径 updated_at 推进到回合结束（活动真正结束），
/// 锚点较旧不拖慢触发；异常路径（零写盘退出）锚点较新，防止逐 tick 热循环。
#[test]
fn idle_baseline_takes_newer_of_anchor_and_updated_at() {
    // 无内存锚点（进程重启后）：回退到磁盘侧 updated_at
    assert_eq!(idle_baseline(None, 1_000), 1_000);
    // 正常路径：updated_at（回合最后一次写盘）较新 → 取 updated_at
    assert_eq!(idle_baseline(Some(2_000), 5_000), 5_000);
    // 异常路径：锚点（触发时刻）较新 → 取锚点（防热循环下限）
    assert_eq!(idle_baseline(Some(8_000), 5_000), 8_000);
    // 相等时取任一即可
    assert_eq!(idle_baseline(Some(5_000), 5_000), 5_000);
}

// ==================== 心跳配置（`HeartbeatConfig`） ====================

#[test]
fn from_metadata_missing_returns_disabled_default() {
    let m = serde_json::json!({ "title": "x" });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(!hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
    assert!(hb.prompt.is_empty());
}

#[test]
fn from_metadata_parses_full_config() {
    let m = serde_json::json!({
        "heartbeat": {
            "enabled": true,
            "interval_seconds": 120,
            "prompt": "请检查待办",
            "include_history": false
        }
    });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 120);
    assert_eq!(hb.prompt, "请检查待办");
    assert!(!hb.include_history);
}

#[test]
fn from_metadata_missing_fields_use_defaults() {
    let m = serde_json::json!({ "heartbeat": { "enabled": true } });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
}

// ==================== 自主触发落格（S9 第 21 步） ====================
//
// 这一段钉的不是"心跳会不会开跑一轮"（那要真模型，归 e2e），而是**触发先落事实**
// 这条链子：`system.triggered` 必须存在且溯源指向更早的格子、健康自检紧随其后、
// 「欲」永远入格、任务却**只在闸门放行时**才长出来。分开断言是因为三者各自的失效
// 都不报错：漏了触发 = 自主行为查无此事；漏了自检 = `system` 实体只开不进；
// 漏了闸门 = 自己写自己的对话（S12 §5「自主写入对话 0」）。

use crate::plugins::session::plugin::SessionConfig;
use crate::symbio_core::{check_all, Entity, Event, Verb, EVENT_USER_MESSAGE};

fn autonomy_plugin(conation_enabled: bool) -> (tempfile::TempDir, Arc<SessionPlugin>) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = Arc::new(SessionPlugin::new(
        None,
        SessionConfig {
            conation_enabled,
            ..SessionConfig::default()
        },
        crate::symbio_core::PluginDir::at(dir.path(), "session"),
    ));
    (dir, plugin)
}

fn wal(p: &SessionPlugin, sid: &str) -> std::path::PathBuf {
    crate::plugins::session::paths::session_dir(&p.storage_dir(), sid)
        .join(crate::plugins::session::paths::V2_WAL_FILE)
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

/// 闸门打开后长目标**只开一次**：心跳按 `interval_seconds` 周期性表达同一条欲，
/// 每次都开格会让就绪集无界增长——而 readyset 是模型唯一的调度候选来源，那就等于
/// 用自己刷爆自己的提示词。而「欲」是流不是状态，照样每 tick 入格。
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
        "「欲」每 tick 照样入格（它是流，不是状态）"
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

    // 收束期写方 `record_to_wal` 跑在 `is_working` 复位之前 ⇒ 忙窗里让开。
    state.inner.write().await.is_working = true;
    assert!(
        !p.record_autonomous_trigger(sid, 100, "看一眼").await,
        "收束期写方在跑 ⇒ 让开"
    );
    assert_eq!(read_wal(&p, sid).len(), 1, "忙时一格都不写");

    // 收件箱的空闲写方只在 `preempt != None` 时才落格 ⇒ 未结清时让开。
    state.inner.write().await.is_working = false;
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
