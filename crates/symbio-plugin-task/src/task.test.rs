//! 任务能力包验收：
//! **清单声明 → 事实 → 三个消费方**的每一环都要看得见，且每条口径各配一个反向用例
//! （声明不变 ⇒ 不补事件；降级 ⇒ 不产事实；成环 ⇒ `invariants` 必须报出来）。

use super::{
    prompt_section, ScheduleContributor, ScheduleDecider, TaskItem, TaskStatus, TaskTable,
};
use symbio_core::PRINCIPAL_SCHEDULER;
use symbio_core::{
    actor_can_assign_work, check_all, readyset_for, Budget, Entity, Event, EventDecl, EventStore,
    Pattern, Scope, Seq, Store, Verb, PRINCIPAL_MAIN,
};
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{
    BeforeDecision, BeforeReadings, BeforeRequest, EffectEnv, EffectInput, ToolBehavior,
};
use symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED;
use symbio_plugin_api::turn::{TurnContributor, TurnEnd, TurnFace};

use crate::decl::{
    EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
    EVENT_TASK_READY, EVENT_TASK_REWORK_CREATED, TASK_VOCAB,
};
use symbio_testkit::trivial_actor_spec;

/// 判据的词汇面：本 crate 的声明表（`submit_declarations!`）链接进来即生效。
///
/// 不直接写 `symbio_plugin_api::vocab()` 是让每条用例读同一个名字——词汇面的出处
/// 若换（例如加一层装配注入），改这一处。
fn guard_vocab() -> &'static [EventDecl] {
    symbio_plugin_api::vocab()
}

fn item(id: &str, goal: &str, deps: &[&str], status: TaskStatus) -> TaskItem {
    TaskItem {
        id: id.to_string(),
        goal: goal.to_string(),
        depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
        status,
    }
}

/// 一次声明的清单（= 一次 `todo_write` 调用里的 `todos[]`）。
struct Decl(Vec<TaskItem>);

fn decl(items: Vec<TaskItem>) -> Decl {
    Decl(items)
}

/// 一项声明 → `todo_write` 的参数形状（生产里由工具交出来，测试里由这里拼）。
fn args_of(items: &[TaskItem]) -> serde_json::Value {
    serde_json::json!({
        "todos": items
            .iter()
            .map(|i| {
                serde_json::json!({
                    "id": i.id,
                    "content": i.goal,
                    "status": match i.status {
                        TaskStatus::Pending => "pending",
                        TaskStatus::InProgress => "in_progress",
                        TaskStatus::Completed => "completed",
                    },
                    "depends_on": i.depends_on,
                })
            })
            .collect::<Vec<_>>()
    })
}

/// 一份**合法**压缩快照里的节点（与 `turn_exec` 落格的形状同构）。
///
/// 快照要真能被 `mutations_latest_snapshot` 认出来，「现行窗口」那类判据才不是空的。
fn snapshot_node(id: &str) -> symbio_core::chat_node::ChatNode {
    use symbio_core::chat_node::{MessageContent, NodeKind, NodeStatus};
    symbio_core::chat_node::ChatNode {
        id: id.to_string(),
        // 不写 `role`：快照的判据是**身份列表**（id 序列），role 的权威在 `principal`。
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(format!("[CONTEXT SNAPSHOT]\n{id}"))),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    }
}

/// 一次「收束」级别的写入：取现有切片 → 按当前轮号入格。
/// `turn` 每调一次递增（与真实桥的行为同形：一轮一格）。
///
/// 走的是**生产那条通路**（DE-025 D-207）：拼一次调用的纯值 → 交效果端口问
/// 能力包要草稿 → 经唯一写入口落格。测试因此同时钉住两件事：任务的成格判据，
/// 以及「这条效果确实由 `task_table` 的声明选出来」。
fn write_all(store: &EventStore, turn: u64, decls: &[Decl]) {
    // 任务事件的溯源锚 = 本轮用户格。夹具此前写死 `0`：日志从 0 起时那条边
    // 指向**自己**（I2 只查非空所以一直是绿的）。没有根就先种一条，锚点取它。
    let anchor = match store.range(Seq::new(0)).first() {
        Some(e) => e.seq.map(|s| s.value()).unwrap_or(0),
        None => store
            .append(Event::pending(
                format!("u-{turn}"),
                symbio_plugin_api::events::EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                symbio_core::PRINCIPAL_MAIN,
            ))
            .expect("根事件入格")
            .value(),
    };
    let items: Vec<TaskItem> = decls.iter().flat_map(|d| d.0.clone()).collect();
    let input = EffectInput {
        tool: "local/todo_write".into(),
        call_id: format!("call-{turn}"),
        args: args_of(&items),
        success: true,
        result: String::new(),
        why: String::new(),
        declared: &[behaviors::TASK_TABLE],
    };
    let snapshot = store.range(Seq::new(0));
    let env = EffectEnv {
        turn,
        user_seq: anchor,
        principal: symbio_core::PRINCIPAL_MAIN,
        source: symbio_core::PRINCIPAL_USER,
        snapshot: &snapshot,
        applied: &[],
    };
    for draft in symbio_plugin_api::effects::apply(&input.call(&env)) {
        store
            .append(draft.into_event(turn, symbio_core::PRINCIPAL_MAIN))
            .expect("任务事件入格");
    }
}

fn kinds(store: &EventStore) -> Vec<String> {
    store
        .range(Seq::new(0))
        .iter()
        .map(|e| e.kind.clone())
        .collect()
}

/// 正向：三项清单各按状态落格——进行中补 `progress`、已完成补 `asserted`，
/// 且 `check_all` 全绿（`task.asserted` 是断言类事件，溯源必须在）。
#[test]
fn declaration_lands_opened_progress_and_asserted() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![
            item("t1", "分析架构", &[], TaskStatus::InProgress),
            item("t2", "写实现", &["t1"], TaskStatus::Pending),
            item("t3", "写文档", &[], TaskStatus::Completed),
        ])],
    );

    let snapshot = store.range(Seq::new(0));
    // 只看任务格：日志里还有一条根事件（用户格），它的存在与任务语义无关。
    let tasks: Vec<&symbio_core::Event> = snapshot
        .iter()
        .filter(|e| e.kind.starts_with("task."))
        .collect();
    assert_eq!(
        tasks.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec![
            EVENT_TASK_OPENED,
            EVENT_TASK_PROGRESS,
            EVENT_TASK_OPENED,
            EVENT_TASK_OPENED,
            EVENT_TASK_ASSERTED,
        ],
        "t1 开 + 推进、t2 开（挂 t1）、t3 开 + 断言"
    );
    let opened = tasks[0];
    assert_eq!(
        opened
            .payload
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0)
    );
    assert_eq!(
        tasks[2].payload.get("depends_on").cloned(),
        Some(serde_json::json!(["t1"])),
        "任务图是数据：depends_on 原样入格"
    );
    assert!(opened.produced_by.is_some(), "断言类事件无溯源即违规（I2）");
    assert!(
        check_all(guard_vocab(), &snapshot).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &snapshot)
    );
}

/// 就绪集 = 非终态 ∧ 依赖全部终态：t1 / t3 就绪、t2 等 t1、t3 自己已终态。
#[test]
fn readyset_lists_dependency_closed_tasks_only() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![
            item("t1", "分析架构", &[], TaskStatus::InProgress),
            item("t2", "写实现", &["t1"], TaskStatus::Pending),
            item("t3", "写文档", &[], TaskStatus::Pending),
        ])],
    );

    let snapshot = store.range(Seq::new(0));
    let ready = readyset_for(TASK_VOCAB, usize::MAX)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value
        .ready;
    let ids: Vec<&str> = ready.iter().map(|t| t.task_id.as_str()).collect();
    assert_eq!(ids, vec!["t1", "t3"], "t2 依赖 t1（未终态）⇒ 不就绪");

    let section = prompt_section(&snapshot, usize::MAX).expect("有就绪任务 ⇒ 有调度段");
    assert!(section.contains("t1") && section.contains("t3"));
    assert!(
        !section.contains("t2"),
        "调度段只念候选集，不把没就绪的任务混进去：{section}"
    );
}

/// 反向：依赖成环 ⇒ `check_all` 必须报（C14），且就绪集诚实为空
/// （跑不完 ≠ 假装能跑）——这正是「断言进 CI」要接的那一条。
#[test]
fn a_dependency_cycle_is_flagged_and_yields_an_empty_readyset() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![
            item("a", "甲", &["b"], TaskStatus::Pending),
            item("b", "乙", &["a"], TaskStatus::Pending),
        ])],
    );

    let snapshot = store.range(Seq::new(0));
    let violations = check_all(guard_vocab(), &snapshot);
    assert!(
        violations.iter().any(|v| v.why.contains("依赖环")),
        "成环必须被断言抓到：{violations:?}"
    );
    let ready = readyset_for(TASK_VOCAB, usize::MAX)
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value
        .ready;
    assert!(ready.is_empty(), "环 ⇒ 依赖永不闭合 ⇒ 无就绪任务");
}

/// 反向：依赖指向不存在的任务 ⇒ 同样是 C14（悬空依赖不是「还没好」，是写错了）。
#[test]
fn a_dangling_dependency_is_flagged() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![item(
            "a",
            "甲",
            &["不存在"],
            TaskStatus::Pending,
        )])],
    );

    let violations = check_all(guard_vocab(), &store.range(Seq::new(0)));
    assert!(
        violations
            .iter()
            .any(|v| v.why.contains("依赖环") || v.why.contains("不存在")),
        "{violations:?}"
    );
}

/// 口径「只记声明」：同一份清单再声明一遍 ⇒ **零新事件**（没有新事实就没有新格）。
#[test]
fn an_unchanged_declaration_writes_nothing() {
    let store = EventStore::new();
    let decls = [decl(vec![
        item("t1", "甲", &[], TaskStatus::InProgress),
        item("t2", "乙", &["t1"], TaskStatus::Pending),
    ])];
    write_all(&store, 0, &decls);
    let before = kinds(&store).len();
    write_all(&store, 1, &decls);
    assert_eq!(kinds(&store).len(), before, "没变化 ⇒ 没有新事实");
}

/// 口径「降级不产事实」（反向用例）：进行中 → 未开始既不写事件、也不改观察状态，
/// 跨轮因此稳定——不会每次声明都补一条。
#[test]
fn downgrading_to_pending_writes_nothing() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::InProgress)])],
    );
    let before = kinds(&store).len();

    write_all(
        &store,
        1,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::Pending)])],
    );
    assert_eq!(kinds(&store).len(), before, "降级不是事实");

    // 下一轮回到进行中：观察状态仍是「进行中」⇒ 同样不补第二条 progress。
    write_all(
        &store,
        2,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::InProgress)])],
    );
    assert_eq!(kinds(&store).len(), before, "来回横跳不产生事件堆积");
}

/// 口径「回退出终态 = 返工」：`completed → in_progress` 不回滚旧节点，
/// 而是新增一条判定事实 + 一个新节点（返工做的是同一件事，前置条件继承）。
#[test]
fn reopening_a_completed_task_opens_a_rework_node() {
    let store = EventStore::new();
    // 前置任务先开格——否则 `t1` 对它的依赖是悬空的，`check_all` 会先报 C14。
    write_all(
        &store,
        0,
        &[decl(vec![
            item("t0", "前置", &[], TaskStatus::Pending),
            item("t1", "甲", &["t0"], TaskStatus::Completed),
        ])],
    );
    write_all(
        &store,
        1,
        &[decl(vec![item(
            "t1",
            "甲",
            &["t0"],
            TaskStatus::InProgress,
        )])],
    );

    let snapshot = store.range(Seq::new(0));
    let rework: Vec<_> = snapshot
        .iter()
        .filter(|e| e.kind == EVENT_TASK_REWORK_CREATED)
        .collect();
    assert_eq!(rework.len(), 1, "回退出终态 = 一次返工");
    assert_eq!(
        rework[0].payload.get("replaces").and_then(|v| v.as_str()),
        Some("t1")
    );
    assert_eq!(
        rework[0].payload.get("round").and_then(|v| v.as_u64()),
        Some(1)
    );
    let node = rework[0]
        .payload
        .get("task_id")
        .and_then(|v| v.as_str())
        .expect("返工节点 id");
    assert_eq!(node, "t1-r1");
    // 返工节点开出来，且继承被返工任务的依赖（返工做的是同一件事）。
    let node_open = snapshot
        .iter()
        .find(|e| {
            e.kind == EVENT_TASK_OPENED
                && e.payload.get("task_id").and_then(|v| v.as_str()) == Some(node)
        })
        .expect("返工节点必须开格（否则它不在调度候选集里）");
    assert_eq!(
        node_open.payload.get("depends_on").cloned(),
        Some(serde_json::json!(["t0"])),
        "返工节点继承依赖"
    );
    assert!(
        check_all(guard_vocab(), &snapshot).is_empty(),
        "一轮返工在界内（`MAX_REWORK_ROUNDS` = 3）：{:?}",
        check_all(guard_vocab(), &snapshot)
    );
}

/// 反向：返工轮数越过上界 ⇒ `check_all` 必须报——`rework_bounded`
/// 有生产写方，不是一条永远空转的断言。
#[test]
fn exceeding_the_rework_bound_is_flagged() {
    let store = EventStore::new();
    // 第 1 轮完工。
    write_all(
        &store,
        0,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::Completed)])],
    );
    // 4 次「回退 → 再完工」⇒ 4 次返工 > 上界 3。
    for round in 1..=4u64 {
        write_all(
            &store,
            round,
            &[decl(vec![item("t1", "甲", &[], TaskStatus::InProgress)])],
        );
        write_all(
            &store,
            round + 4,
            &[decl(vec![item("t1", "甲", &[], TaskStatus::Completed)])],
        );
    }

    let snapshot = store.range(Seq::new(0));
    assert_eq!(
        snapshot
            .iter()
            .filter(|e| e.kind == EVENT_TASK_REWORK_CREATED)
            .count(),
        4
    );
    let violations = check_all(guard_vocab(), &snapshot);
    assert!(
        violations.iter().any(|v| v.why.contains("返工")),
        "返工越过上界必须被断言抓到：{violations:?}"
    );
}

/// 调度段门控（反向）：没有事实源 / 就绪集空 ⇒ 段落**整段不出现**，
/// 而不是发一个空占位给模型。
#[test]
fn prompt_section_is_absent_when_nothing_is_ready() {
    assert!(
        prompt_section(&[], usize::MAX).is_none(),
        "没有事实源 ⇒ 无段落"
    );

    // 只有已终态的任务 ⇒ 就绪集空 ⇒ 同样无段落。
    let done = EventStore::new();
    write_all(
        &done,
        0,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::Completed)])],
    );
    assert!(
        prompt_section(&done.range(Seq::new(0)), usize::MAX).is_none(),
        "全终态 ⇒ 无候选 ⇒ 不给模型一段空调度"
    );
}

/// **压缩不缩小就绪集**（[DE-001 §4.3](../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 一源）：
/// 任务开在压缩之前 ⇒ 调度段仍要念得出它。就绪集是**状态**口径
/// （「压缩前开的、还没做完」= 现在还没做完），不是「最近没被取代的那些事件」。
///
/// 存储层归档（被取代的前缀移进归档段）不改变 `Store::range` 的「完整史」语义
/// （[D-085](../../symbio-plugin-session/README.md#d-085-压缩取代的是消息列表不是轮次生命周期恢复锚读完整史归档刀口不动 "crates/symbio-plugin-session/src/README")；归档自身的判据在
/// `session::store::wal.test.rs`）。本用例因此只需一份「较早任务格 + 较晚快照」的完整史。
///
/// **反向自检**：把 [`prompt_section`] 的切片换成「从最新快照起自切窗口」
/// （`session::store::current_window(...)`）⇒ 本用例红（`prompt_section` 返回 `None`——
/// 那条任务在模型视野里静默消失，而事实源里它一直开着）。
#[test]
fn a_ready_task_survives_the_snapshot_boundary() {
    let store = EventStore::new();
    write_all(
        &store,
        0,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::Pending)])],
    );
    // 压缩：落一条快照（取代此前的一切）。
    store
        .append(
            Event::pending(
                "v2sc-test-archived-readyset",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                0,
                symbio_core::PRINCIPAL_MAIN,
            )
            .with_payload(serde_json::json!({
                // 载荷是**合法**的快照（一份真的 `ChatNode` 列表）：不合法的话
                // `mutations_latest_snapshot` 不认它，「只切现行窗口」那种改法就不会
                // 让本用例红——反向自检要成立，基线必须是真的。
                "messages": [snapshot_node("snap-1")],
                "turns": 1,
            })),
        )
        .expect("快照入格");

    let section = prompt_section(&store.range(Seq::new(0)), usize::MAX)
        .expect("压缩不缩小就绪集 ⇒ 仍有调度段");
    assert!(
        section.contains("t1"),
        "压缩前开的那条任务必须在候选集里：{section}"
    );
}

// ── 调度者与租约（D-189：实现随唯一消费方住本插件） ──

/// 开一个任务（载荷 `{ task_id, depends_on, goal }`）——生产写方见本模块 `TaskTable`。
fn open_task(store: &EventStore, id: &str, depends_on: &[&str], actor: &str) {
    store
        .append(
            Event::pending(
                format!("o-{id}"),
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                0,
                actor,
            )
            .with_payload(
                serde_json::json!({ "task_id": id, "depends_on": depends_on, "goal": "goal" }),
            ),
        )
        .expect("开任务必入库");
}

/// 落一条 `task.assigned`（租约）——只在测试里手写，生产写方是调度者。
fn assign_task(store: &EventStore, id: &str, owner: &str, until_ms: i64) {
    store
        .append(
            Event::pending(
                format!("as-{id}-{until_ms}"),
                EVENT_TASK_ASSIGNED,
                Entity::Task,
                Verb::Asserted,
                0,
                PRINCIPAL_SCHEDULER,
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({
                "task_id": id,
                "owner": owner,
                "lease_ms": until_ms,
                "until_ms": until_ms,
            })),
        )
        .expect("租约入格");
}

/// 调度者是 `Decider` / 80ms / 根作用域，且**持 `assign.work`**（授权表那一行）。
#[test]
fn the_scheduler_spec_is_a_decider_with_assign_work() {
    let spec = ScheduleDecider::spec();
    assert_eq!(spec.pattern, Pattern::Decider);
    assert_eq!(spec.budget_ms, 80, "反射档");
    assert_eq!(spec.scope, Scope::Root);
    assert!(actor_can_assign_work(&spec), "调度者行持 assign.work");
}

/// 100 次注入同一笔就绪任务 ⇒ 只发一笔租约（抢同一份活有仲裁）。
#[test]
fn a_hundred_injections_yield_exactly_one_lease() {
    let store = EventStore::new();
    open_task(&store, "a", &[], PRINCIPAL_SCHEDULER);
    let spec = ScheduleDecider::spec();

    let mut assigned = 0usize;
    let mut ready_facts = 0usize;
    let mut now = 1_000i64;
    for _ in 0..100 {
        let snap = store.range(Seq::new(0));
        let view = readyset_for(TASK_VOCAB, 1)
            .apply(&snap, now, Budget::generous())
            .value;
        let facts = ScheduleDecider.decide(&spec, &view, &snap, PRINCIPAL_SCHEDULER, now);
        for f in &facts {
            if f.kind == EVENT_TASK_ASSIGNED {
                assigned += 1;
            }
            if f.kind == EVENT_TASK_READY {
                ready_facts += 1;
            }
        }
        for (n, f) in facts.into_iter().enumerate() {
            let verb = if f.kind == EVENT_TASK_READY {
                Verb::Progressed
            } else {
                Verb::Asserted
            };
            store
                .append(
                    Event::pending(
                        format!("s-{now}-{n}"),
                        f.kind,
                        Entity::Task,
                        verb,
                        0,
                        PRINCIPAL_SCHEDULER,
                    )
                    .with_produced_by(0)
                    .with_payload(f.payload),
                )
                .expect("调度事实入格");
        }
        now += 10;
    }
    assert_eq!(assigned, 1, "同一笔活被注入 100 次，只发一笔租约");
    assert_eq!(ready_facts, 1, "就绪事实同一周期只落一次（不刷屏）");
}

/// 租约到期 ⇒ 调度者**判死并补落** `task.ready{reason: lease_expired}`，然后重派。
#[test]
fn an_expired_lease_is_judged_dead_and_reassigned() {
    let store = EventStore::new();
    open_task(&store, "a", &[], PRINCIPAL_SCHEDULER);
    assign_task(&store, "a", PRINCIPAL_SCHEDULER, 1_000);
    let spec = ScheduleDecider::spec();
    let snap = store.range(Seq::new(0));

    // 到点：候选集回来了，调度者先判死、再发放。
    let view = readyset_for(TASK_VOCAB, 1)
        .apply(&snap, 1_000, Budget::generous())
        .value;
    let facts = ScheduleDecider.decide(&spec, &view, &snap, PRINCIPAL_SCHEDULER, 1_000);
    let death = facts
        .iter()
        .find(|f| f.kind == EVENT_TASK_READY)
        .expect("到期未判死 ⇒ 回放里它只是「突然又能派了」");
    assert_eq!(death.payload["reason"], "lease_expired");
    assert_eq!(death.payload["expired_owner"], PRINCIPAL_SCHEDULER);
    assert!(
        facts.iter().any(|f| f.kind == EVENT_TASK_ASSIGNED),
        "判死之后同一轮重派"
    );
}

/// 写侧闸：没有 `assign.work` 的主体**一条事实都不写**（fail-closed）。
#[test]
fn a_spec_without_assign_work_writes_nothing() {
    let store = EventStore::new();
    open_task(&store, "a", &[], PRINCIPAL_SCHEDULER);
    let snap = store.range(Seq::new(0));
    let view = readyset_for(TASK_VOCAB, 1)
        .apply(&snap, 9_999, Budget::generous())
        .value;

    // 只持 `produce.artifact` 的工人主体：能产物，不能派活。
    let speaker = trivial_actor_spec("session/worker");
    assert!(
        ScheduleDecider
            .decide(&speaker, &view, &snap, PRINCIPAL_SCHEDULER, 9_999)
            .is_empty(),
        "缺 assign.work ⇒ 零事实（没有资格的调度不留痕）"
    );
}

// ==================== 效果端口：任务表这条效果自己交出草稿（DE-025 D-207） ====================
//
// 判据四条，各钉一条反向：**只认成功的调用**（失败没改清单）、**参数形状容错**
// （缺 `todos` / 空 `content` 不是一份清单）、**同一轮两次声明互不重开**（`applied`
// 那片历史的作用——少了它，第二条声明会把第一条刚开的格再开一次）、**幂等键不撞**
// （同轮两次调用各算各的序号）。

/// 一次调用的效果输入（生产里由 `record_effect` 从注册表元数据抄出 `declared`）。
fn effect(call_id: &str, args: serde_json::Value, success: bool) -> EffectInput {
    EffectInput {
        tool: "local/todo_write".into(),
        call_id: call_id.into(),
        args,
        success,
        result: String::new(),
        why: if success {
            String::new()
        } else {
            "被拒".into()
        },
        declared: &[behaviors::TASK_TABLE],
    }
}

/// 走一次效果端口：拿草稿、落格。返回本次落下的 kind 序列。
fn apply_effects(
    store: &EventStore,
    turn: u64,
    anchor: u64,
    inputs: &[EffectInput],
) -> Vec<String> {
    let mut applied: Vec<Event> = Vec::new();
    let mut kinds = Vec::new();
    for input in inputs {
        let snapshot = store.range(Seq::new(0));
        let env = EffectEnv {
            turn,
            user_seq: anchor,
            principal: symbio_core::PRINCIPAL_MAIN,
            source: symbio_core::PRINCIPAL_USER,
            snapshot: &snapshot,
            applied: &applied,
        };
        for draft in symbio_plugin_api::effects::apply(&input.call(&env)) {
            kinds.push(draft.kind.clone());
            let event = draft.into_event(turn, symbio_core::PRINCIPAL_MAIN);
            store.append(event.clone()).expect("任务事件入格");
            applied.push(event);
        }
    }
    kinds
}

/// 铺一条本轮开口格（效果输入的溯源锚）。
fn seed_anchor(store: &EventStore) -> u64 {
    store
        .append(Event::pending(
            "u-0",
            symbio_plugin_api::events::EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            symbio_core::PRINCIPAL_MAIN,
        ))
        .expect("根事件入格")
        .value()
}

/// 失败的调用不产草稿：`todo_write` 拒掉坏清单时，「模型试过什么」不是清单状态。
#[test]
fn a_failed_declaration_yields_no_draft() {
    let store = EventStore::new();
    let anchor = seed_anchor(&store);
    let kinds = apply_effects(
        &store,
        0,
        anchor,
        &[effect(
            "call-1",
            serde_json::json!({ "todos": [{ "content": "甲" }] }),
            false,
        )],
    );
    assert!(kinds.is_empty(), "失败 ⇒ 没有声明");
}

/// 参数形状容错：缺 `todos`、`todos` 不是数组、项里 `content` 为空 —— 都不是清单。
#[test]
fn payloads_that_cannot_be_a_task_list_yield_no_draft() {
    let store = EventStore::new();
    let anchor = seed_anchor(&store);
    for args in [
        serde_json::json!({}),
        serde_json::json!({ "todos": "不是数组" }),
        serde_json::json!({ "todos": [{ "content": "" }] }),
        serde_json::json!({ "todos": [] }),
    ] {
        let kinds = apply_effects(&store, 0, anchor, &[effect("call-1", args.clone(), true)]);
        assert!(kinds.is_empty(), "{args} 不是一份清单");
    }
}

/// 声明项的读法：`id` 缺席按位次兜底、`status` 三档、`depends_on` 省略即空。
#[test]
fn declared_items_follow_the_tool_side_shape() {
    let store = EventStore::new();
    let anchor = seed_anchor(&store);
    let kinds = apply_effects(
        &store,
        0,
        anchor,
        &[effect(
            "call-1",
            serde_json::json!({
                "todos": [
                    { "content": "分析架构", "status": "in_progress" },
                    { "id": "t2", "content": "写实现", "status": "completed", "depends_on": ["task_1"] },
                    { "id": "t3", "content": "写文档" }
                ],
                "merge": false
            }),
            true,
        )],
    );
    assert_eq!(
        kinds,
        vec![
            EVENT_TASK_OPENED,
            EVENT_TASK_PROGRESS,
            EVENT_TASK_OPENED,
            EVENT_TASK_ASSERTED,
            EVENT_TASK_OPENED,
        ],
        "t1 开 + 推进、t2 开 + 断言、t3 只开"
    );
    let snap = store.range(Seq::new(0));
    let ids: Vec<&str> = snap
        .iter()
        .filter(|e| e.kind == EVENT_TASK_OPENED)
        .filter_map(|e| e.payload.get("task_id").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(ids, vec!["task_1", "t2", "t3"], "id 缺席按位次兜底");
}

/// **同一轮两次声明**：第二条看得见第一条刚落的格，不会把它再开一次。
///
/// 反向自检：把 `EffectEnv::applied` 换成空切片 ⇒ 本条红（`task.opened` 出现两次）。
/// 那不是理论风险——并行工具调用在一轮里叫两次 `todo_write` 是常态。
#[test]
fn a_second_declaration_in_the_same_turn_sees_the_first() {
    let store = EventStore::new();
    let anchor = seed_anchor(&store);
    let kinds = apply_effects(
        &store,
        0,
        anchor,
        &[
            effect(
                "call-1",
                serde_json::json!({ "todos": [{ "id": "t1", "content": "甲", "status": "in_progress" }] }),
                true,
            ),
            effect(
                "call-2",
                serde_json::json!({ "todos": [{ "id": "t1", "content": "甲", "status": "completed" }] }),
                true,
            ),
        ],
    );
    assert_eq!(
        kinds,
        vec![EVENT_TASK_OPENED, EVENT_TASK_PROGRESS, EVENT_TASK_ASSERTED],
        "第二次声明只补状态推进，不重开格"
    );
}

/// 同一轮两次声明的**幂等键不撞**（各算各的序号，调用 id 在号里）。
#[test]
fn event_ids_stay_unique_across_two_calls_in_one_turn() {
    let store = EventStore::new();
    let anchor = seed_anchor(&store);
    apply_effects(
        &store,
        0,
        anchor,
        &[
            effect(
                "call-1",
                serde_json::json!({ "todos": [{ "id": "t1", "content": "甲" }] }),
                true,
            ),
            effect(
                "call-2",
                serde_json::json!({ "todos": [{ "id": "t2", "content": "乙" }] }),
                true,
            ),
        ],
    );
    let snap = store.range(Seq::new(0));
    let mut ids: Vec<&str> = snap.iter().map(|e| e.event_id.as_str()).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "幂等键撞了 ⇒ 第二条被存储静默丢掉");
}

// ==================== 写侧判断（DE-033 §3 / DE-025 D-219） ====================
//
// 判据住在**本插件的 `TaskTable::before`**：工具声明 `task_table`，宿主把批首读好的
// 主体交进来，本实现只答「这个主体能不能写」。名字与主体的匹配在 `ToolDecls`
// （`decls.test.rs`），这里只钉资格本身。

fn judgement<'a>(args: &'a serde_json::Value, principal: &'a str) -> BeforeRequest<'a> {
    BeforeRequest {
        tool: "local/todo_write",
        call_id: "call-1",
        args,
        principal,
        declared: &[behaviors::TASK_TABLE],
        facts: &[],
        readings: BeforeReadings::default(),
    }
}

/// `define.work` 的三个持有者与 `assert.verification` 的两个持有者都能写任务表。
///
/// 立任务（主智能体 / 自主发起者 / 规划者）与独立判定（仲裁者 / 验证者）是任务表上的
/// 两种权威；两项互不可写（C15），所以「谁能写」必须按**任一**能力判。
#[test]
fn role_holders_may_write_the_task_table() {
    let args = serde_json::json!({});
    for agent in ["main", "autonomous", "planner", "arbiter", "verifier"] {
        let principal = symbio_core::principal_of(Some(agent));
        assert!(
            matches!(
                TaskTable.before(&judgement(&args, &principal)),
                BeforeDecision::Allow
            ),
            "主体 `{principal}` 持权威，必须能写任务表"
        );
    }
}

/// 平凡值（无 `AGENT_ID` ⇒ `session/main`）照常放行——生产里的绝对多数会话是这一形态。
#[test]
fn the_trivial_session_may_write_the_task_table() {
    let args = serde_json::json!({});
    assert!(matches!(
        TaskTable.before(&judgement(&args, PRINCIPAL_MAIN)),
        BeforeDecision::Allow
    ));
}

/// 纯产出 / 纯派活 / 纯判定主体被拒——C15 的精神在任务表这一层。
///
/// 工人与对等体只持 `produce.artifact`（交付物是产物格），市场只持 `assign.work`
/// （派活 ≠ 立活），守门者与抢占判定者只持 `judge.intent`。今天没有生产路径让这些
/// 主体调任务表——此判据防的是**未来**：角色行落地后，一个绑 `worker` 的子会话被
/// 诚实拒止，而不是静默以主智能体资格写下去。
#[test]
fn producer_only_principals_are_refused_the_task_table() {
    let args = serde_json::json!({});
    for agent in ["worker", "peer-b", "market", "gatekeeper", "reflex"] {
        let principal = symbio_core::principal_of(Some(agent));
        let BeforeDecision::Refuse(refusal) = TaskTable.before(&judgement(&args, &principal))
        else {
            panic!("主体 `{principal}` 不持权威，必须被拒");
        };
        assert_eq!(refusal.gate, "任务表");
        assert_eq!(refusal.kind, symbio_core::failure_kind::PERMISSION_DENIED);
        assert!(
            refusal.reason.contains(&principal)
                && refusal.reason.contains("define.work / assert.verification"),
            "拒因要点名主体与缺的资格（实际: {}）",
            refusal.reason
        );
        assert!(
            !refusal.notes_delegation,
            "工具没跑 ⇒ 「声明过什么」无从说起，不记承诺"
        );
    }
}

/// 未绑定登记的主体在合流面没有行 ⇒ 拒（[D-099] 未登记 = 拒、[D-100] 绑定登记前
/// 没有行）。本用例直读合流面、不经过装配点绑定：忘配 / 忘绑的后果是一条会被看见的
/// 拒绝，不是一次静默超权。
#[test]
fn an_unbound_subagent_is_refused_the_task_table() {
    let args = serde_json::json!({});
    assert!(matches!(
        TaskTable.before(&judgement(&args, "session/reviewer")),
        BeforeDecision::Refuse(_)
    ));
}

// ==================== 轮末贡献：调度不再由宿主代劳（DE-025 D-250） ====================
//
// 三条判据：只走生成轮（答话轮没有新声明）、事件号与归属跟宿主面同形
// （`t{turn}-s{n}` / 调度者 / 本轮开口格）、以及宿主盖章（`turn` / `ts`）由端口给。

/// 一条已就绪的任务 + 本轮开口格。
fn contributor_history(store: &EventStore) -> (u64, Vec<Event>) {
    let anchor = seed_anchor(store);
    open_task(store, "a", &[], PRINCIPAL_SCHEDULER);
    (anchor, store.range(Seq::new(0)))
}

fn turn_end<'a>(
    face: TurnFace,
    history: &'a [Event],
    anchor: &'a str,
    user_seq: u64,
) -> TurnEnd<'a> {
    TurnEnd {
        face,
        turn: 3,
        now_ms: 2_000,
        principal: PRINCIPAL_MAIN,
        user_seq,
        anchor,
        user_text: "做点事",
        response: None,
        task_cap: usize::MAX,
        history,
    }
}

/// 生成轮：就绪集 → `task.ready` + `task.assigned`，号是 `t3-s{n}`、归属是调度者。
#[test]
fn the_contributor_lands_ready_and_assigned_on_a_generated_turn() {
    let store = EventStore::new();
    let (anchor, history) = contributor_history(&store);
    let end = turn_end(TurnFace::Generated, &history, "t3", anchor);

    let drafts = ScheduleContributor.contribute(&end);
    assert_eq!(
        drafts
            .iter()
            .map(|d| d.fact.kind.as_str())
            .collect::<Vec<_>>(),
        vec![EVENT_TASK_READY, EVENT_TASK_ASSIGNED],
        "先记就绪、再发放"
    );
    assert_eq!(
        drafts
            .iter()
            .map(|d| d.fact.key.as_str())
            .collect::<Vec<_>>(),
        vec!["t3-s0", "t3-s1"],
        "事件号词干由宿主给的轮锚拼出（与旧的 `t{{turn}}-s` 同形）"
    );
    for draft in &drafts {
        assert_eq!(draft.actor, Some(PRINCIPAL_SCHEDULER), "派活的主体是调度者");
        assert_eq!(draft.fact.entity, Entity::Task);
        assert_eq!(
            draft.fact.produced_by,
            Some(anchor),
            "溯源锚是本轮开口格（I2）"
        );
    }
}

/// 宿主盖章：`turn` / `ts` 由端口给，能力只声明事实与归属。
#[test]
fn the_host_stamps_turn_and_time() {
    let store = EventStore::new();
    let (anchor, history) = contributor_history(&store);
    let end = turn_end(TurnFace::Generated, &history, "t3", anchor);
    let event = ScheduleContributor.contribute(&end)[0]
        .clone()
        .into_event(&end);
    assert_eq!(event.turn, 3);
    assert_eq!(event.ts, 2_000);
    assert_eq!(event.actor, PRINCIPAL_SCHEDULER);
}

/// 反向：定稿答话轮没有工具交换，也就没有新声明——贡献者一条都不交。
#[test]
fn an_answered_turn_contributes_no_schedule() {
    let store = EventStore::new();
    let (anchor, history) = contributor_history(&store);
    let end = turn_end(TurnFace::Answered, &history, "t3", anchor);
    assert!(
        ScheduleContributor.contribute(&end).is_empty(),
        "答话轮不该开调度账"
    );
}

/// **EX-003 反向用例**：两份实现同时生效必须红。
///
/// 迁出之后，唯一能守住「旧实现没删干净」的装载期判据是 `Gap::DuplicateEvent`：
/// 同名事件在两个 owner 下各声明一次，`seal` 必须把重名报出来。这条判的是**装配口**，
/// 不是本包的私有行为——同一份检查也拦着后来者把任务名再声明第二遍。
#[test]
fn a_second_declaration_of_the_same_task_event_is_rejected() {
    fn decls() -> symbio_core::PluginDeclarations<'static> {
        const EVENTS: &[EventDecl] = &[EventDecl::new(
            EVENT_TASK_OPENED,
            (Entity::Task, Verb::Opened),
        )];
        symbio_core::PluginDeclarations {
            events: EVENTS,
            facts: &[],
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: &[],
        }
    }
    let mut b = symbio_core::Builder::new();
    b.declare("crates/symbio-plugin-task", decls());
    // 旧实现没删干净的样子：同一个名字在第二个 owner 下再声明一次。
    b.declare("crates/symbio-plugin-loop", decls());
    let Err(gaps) = b.seal(&symbio_plugin_api::rows::row_writes()) else {
        panic!("同名事件在两个 owner 下声明，装载必须判红");
    };
    assert!(
        gaps.contains(&symbio_core::ring::Gap::DuplicateEvent {
            name: EVENT_TASK_OPENED
        }),
        "缺口里必须点名 `{EVENT_TASK_OPENED}` 重名：{gaps:?}"
    );
}
