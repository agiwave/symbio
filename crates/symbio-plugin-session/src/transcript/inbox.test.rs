//! `transcript/inbox.rs` 的单元测试（入队 / 取消 / 清空 / 忙则排队 / 抢占落格）。
//!
//! ## 这里**不**测"跑一轮"
//!
//! 真跑一轮要整棵树（能力收集 + 模型 provider），那是 e2e 的活（`e2e/cases/t15-*.mjs`）。
//! 本文件锁的是队列本身的规则：顺序、身份、两种取消的分界、**忙则排队**，以及
//! 忙窗里那条**只判不落、空闲才落格**的抢占链。前几者是 D-026 的核心承诺，
//! 后者是落格时序——它们恰好**能**在单测里钉死，因为判的都是
//! "这一趟要不要动队列 / 动事实源"，与跑得成跑不成无关。

use super::*;
use crate::plugin::SessionConfig;
use symbio_plugin_api::vocab;
// 夹具自己开档写一轮事实：`use super::*` 带不过来（生产侧读走存储层句柄）。
use crate::store::EventWalStore;
use symbio_kernel::event::Seq;
use symbio_sdk::chat::session_chat;

/// 身份写 `principal`（D-147）：夹具模仿的是**落库面**的消息，`role` 只是读侧残留格。
fn principal_for(role: cm::NodeRole) -> symbio_sdk::chat::principal::Principal {
    use symbio_sdk::chat::principal::Principal;
    match role {
        cm::NodeRole::User => Principal::from_legacy(symbio_sdk::authz::PRINCIPAL_USER),
        cm::NodeRole::Assistant => Principal::from_legacy("session/main"),
        cm::NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        cm::NodeRole::System => Principal::from_legacy("system/local"),
    }
}

fn plugin() -> Arc<SessionPlugin> {
    Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        crate::test_dir(),
    ))
}

fn user_message(text: &str) -> cm::ChatNode {
    cm::ChatNode {
        principal: Some(principal_for(cm::NodeRole::User)),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(cm::MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

#[tokio::test]
async fn enqueue_preserves_fifo_order_and_address_identity() {
    let p = plugin();
    p.enqueue_inbox(
        "s1",
        None,
        user_message("第一条"),
        session_chat::Request::default(),
        None,
    )
    .await;
    let second = p
        .enqueue_inbox(
            "s1",
            Some("i2".to_string()),
            user_message("第二条"),
            session_chat::Request::default(),
            None,
        )
        .await;

    // 具名条目：**地址末段即身份**——条目 id 与消息 id 是同一个值，
    // 两处各生成一个会让"按地址取消息"在两套 id 之间对不上
    assert_eq!(second.id, "i2");
    assert_eq!(second.message.id, "i2");

    let items = p.inbox_items("s1").await;
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .map(|i| i.message.content.clone().unwrap().to_text())
            .collect::<Vec<_>>(),
        vec!["第一条".to_string(), "第二条".to_string()],
        "入队顺序就是消费顺序（FIFO）"
    );
    // 未具名的那条：id 由 provider 生成，且与消息 id 一致
    assert_eq!(items[0].id, items[0].message.id);
    assert!(!items[0].id.is_empty());
}

#[tokio::test]
async fn cancel_and_clear_only_affect_queued_items() {
    let p = plugin();
    for (id, text) in [("i1", "一"), ("i2", "二"), ("i3", "三")] {
        p.enqueue_inbox(
            "s1",
            Some(id.to_string()),
            user_message(text),
            session_chat::Request::default(),
            None,
        )
        .await;
    }

    assert!(p.cancel_inbox_item("s1", "i2").await, "排队中的条目可取消");
    assert!(
        !p.cancel_inbox_item("s1", "i2").await,
        "取消两次是两次动作：第二次找不到（调用方据此报 NotFound，不静默成功）"
    );
    assert!(
        !p.cancel_inbox_item("s1", "nope").await,
        "不存在的条目同样报找不到"
    );

    let ids: Vec<String> = p
        .inbox_items("s1")
        .await
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids, vec!["i1".to_string(), "i3".to_string()]);

    let cleared = p.clear_inbox("s1").await;
    assert_eq!(cleared, vec!["i1".to_string(), "i3".to_string()]);
    assert!(p.inbox_items("s1").await.is_empty());
}

/// `run_inbox_turn` 的上下文是**自己造的**：只带目标会话与工作目录。
///
/// 发起者的 `SESSION_ID` 刻意不沿用——跨空间写入时那是发起者自己的会话
/// （父会话 → 子智能体空间），沿用会把消息投错会话。这里从**入口**侧验一次：
/// 入队带 `workdir` 时它会出现在条目上（消费者据此还原请求级上下文）。
#[tokio::test]
async fn enqueue_carries_request_params_and_workdir() {
    let p = plugin();
    let params = session_chat::Request {
        mode: Some("auto".to_string()),
        risk_level: Some("high".to_string()),
        agent_id: Some("ag1".to_string()),
        ..Default::default()
    };
    let item = p
        .enqueue_inbox(
            "s1",
            None,
            user_message("走这一份参数"),
            params,
            Some("C:/w".to_string()),
        )
        .await;
    assert_eq!(item.params.mode.as_deref(), Some("auto"));
    assert_eq!(item.params.risk_level.as_deref(), Some("high"));
    assert_eq!(item.params.agent_id.as_deref(), Some("ag1"));
    assert_eq!(item.workdir.as_deref(), Some("C:/w"));
    // 与"哪条消息发给哪个会话"无关的那三个字段不在条目参数里（各只有一个位置）
    assert!(item.params.session_id.is_none());
    assert!(item.params.message.is_none());
    assert!(item.params.resume.is_none());
}

// ────────────── 补充整合：抽干整队（G-A）──────────────
//
// 抽干与合并的**规则**在单测里钉（本段 + `supplements.test.rs`）；
// 「合并后的那条真的进了下一轮 LLM 请求」由 e2e 钉
// （`e2e/cases/t19-supplements-merge.mjs`）——那要读回请求体才看得见。

/// 建一个带自定义配置的插件（抽干上界 / 总开关都在配置里）。
fn plugin_with(config: SessionConfig) -> Arc<SessionPlugin> {
    Arc::new(SessionPlugin::new(None, config, crate::test_dir()))
}

/// 入队 n 条（id 依次 i1..in），返回 id 列表。
async fn enqueue_n(p: &Arc<SessionPlugin>, n: usize) -> Vec<String> {
    let mut ids = Vec::new();
    for k in 1..=n {
        let id = format!("i{k}");
        p.enqueue_inbox(
            "s1",
            Some(id.clone()),
            user_message(&format!("第 {k} 条")),
            session_chat::Request::default(),
            None,
        )
        .await;
        ids.push(id);
    }
    ids
}

/// 抽干取的是**整队**（不是一条），且按 FIFO 顺序。
#[tokio::test]
async fn drain_takes_the_whole_queue_in_one_batch() {
    let p = plugin();
    let ids = enqueue_n(&p, 3).await;
    let state = p.active_mgr.get_or_create("s1").await;

    let batch = p.take_inbox_batch(&state).await;
    let taken: Vec<String> = batch.iter().map(|i| i.id.clone()).collect();

    assert_eq!(taken, ids, "一趟取走整队，且保持入队顺序");
    assert!(
        p.inbox_items("s1").await.is_empty(),
        "整队已被取走，队列应为空"
    );
}

/// 上界：超过 `supplements_max_per_drain` 的部分**留队**，等下一个抽干点。
#[tokio::test]
async fn drain_is_bounded_by_max_per_drain() {
    let p = plugin_with(SessionConfig {
        supplements_max_per_drain: 2,
        ..SessionConfig::default()
    });
    enqueue_n(&p, 3).await;
    let state = p.active_mgr.get_or_create("s1").await;

    let batch = p.take_inbox_batch(&state).await;
    assert_eq!(batch.len(), 2, "一趟至多取上界条");
    assert_eq!(
        p.inbox_items("s1").await.len(),
        1,
        "超出的那条留队，等下一个抽干点"
    );
}

/// **平凡值**：`supplements_enabled = false` ⇒ 一趟只取一条（今天的行为）。
#[tokio::test]
async fn disabled_supplements_take_one_item_per_pass() {
    let p = plugin_with(SessionConfig {
        supplements_enabled: false,
        ..SessionConfig::default()
    });
    enqueue_n(&p, 3).await;
    let state = p.active_mgr.get_or_create("s1").await;

    let batch = p.take_inbox_batch(&state).await;
    assert_eq!(batch.len(), 1, "关掉开关后一趟一条，与今天一致");
    assert_eq!(p.inbox_items("s1").await.len(), 2, "其余留队");
}

/// 订阅**本插件实例**的变更流，收集被通知的地址。
///
/// 抽成函数有两个理由，都不是为了好看：
/// - 与 `plugin/vdfs_provider.test.rs::watch_changes` 同形——同一份手法只留一处定义；
/// - 订阅表是本插件实例的（`SessionPlugin::change_subs`），不是进程级全局总线，
///   因此并行用例互不干扰，也不必按 `session_id` 过滤。
fn watch_inbox_paths(p: &SessionPlugin) -> Arc<std::sync::Mutex<Vec<String>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    p.change_subs.watch(
        "",
        Arc::new(move |c: symbio_kernel::vdfs::VdfsChange| sink.lock().unwrap().push(c.path)),
    );
    seen
}

/// 抽干与删除变更**成对**：每取走一条都要在**它自己的地址**上通知一次。
///
/// 分开写就会出现两种静默不一致：抽了没通知（前端以为条目还在）、
/// 通知了没抽（条目凭空消失）。
#[tokio::test]
async fn drain_notifies_every_taken_item_at_its_own_address() {
    let p = plugin();
    let ids = enqueue_n(&p, 3).await;
    let seen = watch_inbox_paths(&p);

    let state = p.active_mgr.get_or_create("s1").await;
    let batch = p.take_inbox_batch(&state).await;
    assert_eq!(batch.len(), 3);

    let paths = seen.lock().unwrap().clone();
    for id in &ids {
        assert!(
            paths.iter().any(|p| p.ends_with(&format!("inbox/{id}"))),
            "条目 {id} 被取走时必须通知它自己的地址，实得 {paths:?}"
        );
    }
}

/// 上界配成 0 时下界兜到 1：不能"一趟取零条"把队列永久卡死。
#[tokio::test]
async fn zero_bound_is_clamped_to_one() {
    let p = plugin_with(SessionConfig {
        supplements_max_per_drain: 0,
        ..SessionConfig::default()
    });
    enqueue_n(&p, 2).await;
    let state = p.active_mgr.get_or_create("s1").await;

    let batch = p.take_inbox_batch(&state).await;
    assert_eq!(batch.len(), 1, "上界 0 会让队列永不消费，必须兜到 1");
}

// ────────────── 插话抢占：忙窗判、空闲落、收在插话轮之后───────
//
// 上面几例钉的是**队列**规则（忙则排队，D-026）；这一段钉的是跨过忙窗的那条
// 链子。断言一律看**就绪集**：`readyset` 是模型唯一的调度候选来源，挂起没落格、
// 恢复没落格对用户都是「任务凭空消失 / 永不复活」，而两者都**不报错**——
// 正是 DE-016 §9.2 要防的静默失效。

/// 每例独占存储根：抢占要往事实源写格，共用 `test_dir()` 会跨用例串味。
fn preempt_plugin() -> (tempfile::TempDir, Arc<SessionPlugin>) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        symbio_sdk::plugin::PluginDir::at(dir.path(), "session"),
    ));
    (dir, plugin)
}

fn wal_path(p: &SessionPlugin, sid: &str) -> std::path::PathBuf {
    crate::paths::session_dir(&p.storage_dir(), sid).join(crate::paths::V2_WAL_FILE)
}

/// 往该会话的事实源写一批事实（模拟上一轮的轮末收束入格）。
///
/// 目录要自己建：`WalStore::open` 是「写方已经就位」的入口，盘上没这层目录属
/// 存储层灾难、直接 panic（生产里由统一发送链路先落会话目录）。
fn seed_wal(p: &SessionPlugin, sid: &str, facts: Vec<symbio_kernel::event::Event>) {
    let path = wal_path(p, sid);
    std::fs::create_dir_all(path.parent().expect("WAL 路径必有父目录")).expect("会话目录创建失败");
    let store = EventWalStore::open(&path).expect("打开 WAL");
    for fact in facts {
        store.append(fact).expect("事实入格");
    }
}

fn read_wal(p: &SessionPlugin, sid: &str) -> Vec<symbio_kernel::event::Event> {
    EventWalStore::open_readonly(wal_path(p, sid))
        .expect("只读打开 WAL")
        .range(Seq::new(0))
}

/// 一轮的事实。`final_first` = 收束发言在 `task.opened` **之前**——这正是
/// 轮末收束的落笔顺序（收束格先落、派生事实后补；DE-016 §9.1 的判据输入，见那里的
/// 「顺序是判据」注记）。
fn turn_facts(final_first: bool) -> Vec<symbio_kernel::event::Event> {
    use symbio_kernel::event::{Entity, Event, Verb};
    use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};

    let user = Event::pending(
        "u1",
        EVENT_USER_MESSAGE,
        Entity::Scope,
        Verb::Opened,
        0,
        "user:t",
    );
    let final_msg = Event::pending(
        "f1",
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Scope,
        Verb::Closed,
        0,
        "session/t",
    )
    .with_produced_by(0);
    let task = Event::pending(
        "t1",
        symbio_plugin_api::events::EVENT_TASK_OPENED,
        Entity::Task,
        Verb::Opened,
        0,
        "user:t",
    )
    .with_produced_by(0)
    .with_payload(serde_json::json!({ "task_id": "x1", "goal": "把事办了" }));

    if final_first {
        vec![user, final_msg, task]
    } else {
        vec![user, task, final_msg]
    }
}

/// 某一格上落了哪些任务 id（**事实层**的读法，不是就绪集）。
///
/// 就绪集是任务能力包的折叠（名册 #23，`D-289`），本测试二进制不链接它（E-009）——
/// 这里只判 session 这一支**写了什么**：挂起与恢复各自落成哪一格的哪条任务。
/// 「这两格真的让模型看不见 / 重新看见那条任务」归 owner 的单测
/// （`crates/symbio-plugin-task/src/readyset.test.rs`）与 e2e `t34-preemption.mjs`
/// （它读生产二进制的 `readyset.ready`）。在这里复算一份折叠就是替身 owner。
fn task_ids_in(p: &SessionPlugin, sid: &str, verb: symbio_kernel::event::Verb) -> Vec<String> {
    read_wal(p, sid)
        .iter()
        .filter(|e| e.entity == symbio_kernel::event::Entity::Task && e.verb == verb)
        .filter_map(|e| {
            e.payload
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// 事实源里有没有那一格：按 **`entity × verb`** 认（名字表在 core 里，不复述字面量）。
fn has_fact(
    p: &SessionPlugin,
    sid: &str,
    entity: symbio_kernel::event::Entity,
    verb: symbio_kernel::event::Verb,
) -> bool {
    read_wal(p, sid)
        .iter()
        .any(|e| e.entity == entity && e.verb == verb)
}

// ==================== S07 抢占判定（纯函数 + 行，D-190） ====================
//
// 判定表四类结论、预算读的是**行自己的** `budget_ms`，以及挂起 / 恢复**落了哪一格**。
// 这些用例随策略实现从 core 迁来（D-190）——判据没变，只是落点归 owner。
// 「落了这一格之后任务离开就绪集」不是本文件的判据：那是折叠，随它一起归任务能力包
// （名册 #23，`D-289`；组合判据在 e2e `t34-preemption.mjs`）。

use symbio_kernel::actors::{Pattern, Scope};
use symbio_kernel::store::{EventStore, Store};

/// 造一个在跑任务（供插话 / 熔断场景使用）。
fn running_task(store: &EventStore, id: &str) {
    store
        .append(
            Event::pending(
                format!("o-{id}"),
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                0,
                "session/main",
            )
            .with_payload(serde_json::json!({ "task_id": id, "depends_on": [], "goal": "goal" })),
        )
        .expect("开任务必入库");
}

fn facts(store: &EventStore) -> Vec<Event> {
    store.range(symbio_kernel::event::Seq::new(0))
}

/// DE-037 §6 验收 1：注入打断事件 → 80ms 内必须产出处置（Suspend ⇒ task.controlled 落库）。
#[test]
fn interruption_within_reflex_budget_produces_control_event() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let d = PreemptionDecider.decide(&PreemptionDecider.spec(), &facts(&store), 10);
    match d {
        Preemption::Suspend { task_id, as_of_seq } => {
            assert_eq!(task_id, "t1");
            // 挂起就是一条事件 + 一条控制事实（判定者只产控制事件，不发言）。
            store
                .append(PreemptionDecider.held_event(&task_id, as_of_seq, as_of_seq))
                .unwrap();
            store
                .append(PreemptionDecider.control_event("interrupt-suspend", as_of_seq))
                .unwrap();
            let snapshot = facts(&store);
            assert!(
                snapshot
                    .iter()
                    .any(|e| e.kind == symbio_plugin_api::events::EVENT_TASK_CONTROLLED),
                "80ms 内必须有 task.controlled"
            );
            assert!(
                symbio_sdk::invariants::check_all(vocab(), &snapshot).is_empty(),
                "{:?}",
                symbio_sdk::invariants::check_all(vocab(), &snapshot)
            );
        }
        other => panic!("预算内应实质判定，得到 {other:?}"),
    }
}

/// DE-037 §6 验收 4（反向）：同一场景，预算放宽 → 判定结果必须改变（预算参数在生效）。
#[test]
fn widening_budget_changes_preemption_outcome() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let snapshot = facts(&store);
    // 判定耗时 100ms：判定者行（反射档，`spec().budget_ms` = 80）超时 → 默认继续（不挂起）。
    assert_eq!(
        PreemptionDecider.decide(&PreemptionDecider.spec(), &snapshot, 100),
        Preemption::TimeoutDefaultContinue
    );
    // 同一份判据数据，只把**行上的预算**放宽到 60000 → 实质判定成立（挂起）。
    // 断言「`decide` 读的是规格自己的 budget_ms」——不是某个调用方现算的数值。
    let mut generous = PreemptionDecider.spec();
    generous.budget_ms = 60_000;
    assert_eq!(
        PreemptionDecider.decide(&generous, &snapshot, 100),
        Preemption::Suspend {
            task_id: "t1".into(),
            as_of_seq: 0
        }
    );
}

/// DE-016 §9.3 边界：final 已发出后到达插话 → 不抢占，排队（已发出的发言不可撤回）。
#[test]
fn interruption_after_final_is_queued_not_preempted() {
    let store = EventStore::new();
    running_task(&store, "t1");
    store
        .append(
            Event::pending(
                "f0",
                EVENT_CHAT_ASSISTANT_FINAL,
                Entity::Scope,
                Verb::Closed,
                0,
                "session/main",
            )
            .with_produced_by(0),
        )
        .unwrap();
    assert_eq!(
        PreemptionDecider.decide(&PreemptionDecider.spec(), &facts(&store), 10),
        Preemption::Queue
    );
}

/// DE-037 §6 验收 2 的**写侧半边**：抢占留下的挂起 / 恢复必须落在 owner 折叠认的那两格
/// （`task × held` 与 `task × progressed`，载荷都带 `task_id`，挂起那条还带 as-of 锚）。
///
/// 「落了这一格 ⇒ 任务离开就绪集」「as-of 重放与挂起前一致」是**折叠**的判断，归
/// `crates/symbio-plugin-task`（名册 #23，`D-289`）：本测试二进制不链接它（E-009），
/// 在这里复算一份就绪集就是替身 owner，只会把那条真判据变成假绿。插话链上「有没有落格」
/// 仍在本文件判（`interjection_holds_the_running_task_and_releases_it_when_idle`）。
#[test]
fn preemption_events_land_on_the_cells_the_owner_fold_reads() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let t = store.head().value();

    let held = PreemptionDecider.held_event("t1", t, 0).with_ts(5_000);
    assert_eq!(held.kind, EVENT_TASK_HELD, "名字就是声明表里那一格");
    assert_eq!(held.entity, Entity::Task);
    assert_eq!(
        held.verb,
        Verb::Held,
        "挂起是 `held` 那一格，不是 `asserted`"
    );
    assert_eq!(
        held.payload.get("task_id").and_then(|v| v.as_str()),
        Some("t1"),
        "折叠按 `task_id` 认挂起的是哪条活"
    );
    store.append(held).expect("挂起入格");

    let resumed = PreemptionDecider.resume_event("t1", t);
    assert_eq!(resumed.entity, Entity::Task);
    assert_eq!(
        resumed.verb,
        Verb::Progressed,
        "恢复也是一条事件，不是改历史"
    );
    assert_eq!(
        resumed.payload.get("task_id").and_then(|v| v.as_str()),
        Some("t1"),
        "与挂起成对：同一任务的第二次挂起要有第二次恢复"
    );
    store.append(resumed).expect("恢复入格");

    // 挂起与恢复都只是**多一格**：事实源里没有一处状态被搬动（append-only）。
    let events = facts(&store);
    assert_eq!(events.len(), 3, "开格 + 挂起 + 恢复，三条都是新增");
    assert!(
        events.iter().all(|e| e.verb != Verb::Asserted),
        "抢占一支不碰终态：{events:?}"
    );
}

/// DE-037 §5：判定者的生产行只授予 `judge.intent`——无 `reply.*` 写权（不得直接发言）。
///
/// 判据走**生产表**（`production_matrix`），不是自造矩阵：表里少一行 `session/reflex`，
/// 自造矩阵照样绿。表是权威（D-099）。
#[test]
fn preemption_decider_row_grants_only_judge_intent() {
    let grants = symbio_sdk::authz::production_matrix().grant_names_of(PRINCIPAL_REFLEX);
    assert_eq!(
        grants,
        vec!["judge.intent".to_string()],
        "判定者只产控制事件、不得发言（DE-037 §5 的窄行）"
    );
}

/// S07 判定者**行**——五字段取实 + 能力读方。
///
/// 判据分两半：① 行的形状与 DE-037 §3 表逐字相符（五字段全部取实、非平凡值）；
/// ② **能力读方**——`can_judge_intent` 读规格**自己身上**的能力集，含反向用例
/// （清空能力集 ⇒ 写权拒；判定若绕回授权表则这条绿）。
#[test]
fn preemption_decider_spec_carries_its_row_and_gates_on_judge_intent() {
    let spec = PreemptionDecider.spec();
    assert_eq!(
        spec.principal, PRINCIPAL_REFLEX,
        "主体名 = 授权表那一行的键"
    );
    assert_eq!(
        spec.pattern,
        Pattern::Decider,
        "规则判定，非模型（DE-037 §3）"
    );
    assert_eq!(
        spec.capabilities,
        vec!["judge.intent".to_string()],
        "能力取授权表**窄行**"
    );
    assert_eq!(
        spec.budget_ms,
        LatencyTier::Reflex.budget_ms(),
        "预算 = 反射档 80ms"
    );
    assert!(
        matches!(spec.scope, Scope::Root),
        "会话驱动的判定都是根作用域"
    );

    // 能力读方：判定者持 judge.intent ⇒ 可落控制事实（`task.held` / `control`）。
    assert!(
        symbio_sdk::actors::actor_can_judge_intent(&spec),
        "判定者持 judge.intent ⇒ 写侧闸放行"
    );
    // 反向用例：清空能力集 ⇒ 写权拒。判定若绕回授权表，这条会绿（假绿）——它必须红。
    let mut stripped = PreemptionDecider.spec();
    stripped.capabilities.clear();
    assert!(
        !symbio_sdk::actors::actor_can_judge_intent(&stripped),
        "无 judge.intent ⇒ 不得落控制事实（读的是规格自己的集合）"
    );
}
