//! 任务表写方验收（S7 步 16–18，[04 §3.1 批⑨](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
//! **清单声明 → 事实 → 两个消费方**的每一环都要看得见，且每条口径各配一个反向用例
//! （声明不变 ⇒ 不补事件；降级 ⇒ 不产事实；成环 ⇒ `invariants` 必须报出来）。

use super::{prompt_section, write, ScheduleDecider};
use symbio_core::facts::{TaskDeclaration, TaskItem, TaskStatus};
use symbio_core::PRINCIPAL_SCHEDULER;
use symbio_core::{
    actor_can_assign_work, check_all, readyset, Budget, Entity, Event, EventStore, Pattern, Scope,
    Seq, Store, Verb, EVENT_SESSION_CONSOLIDATED, EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED,
    EVENT_TASK_OPENED, EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED,
};

use crate::control_decl::EVENT_TASK_READY;
use symbio_testkit::trivial_actor_spec;

fn item(id: &str, goal: &str, deps: &[&str], status: TaskStatus) -> TaskItem {
    TaskItem {
        id: id.to_string(),
        goal: goal.to_string(),
        depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
        status,
    }
}

fn decl(items: Vec<TaskItem>) -> TaskDeclaration {
    TaskDeclaration { items }
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
fn write_all(store: &EventStore, turn: u64, decls: &[TaskDeclaration]) {
    // 任务事件的溯源锚 = 本轮用户格。夹具此前写死 `0`：日志从 0 起时那条边
    // 指向**自己**（I2 只查非空所以一直是绿的）。没有根就先种一条，锚点取它。
    let anchor = match store.range(Seq::new(0)).first() {
        Some(e) => e.seq.map(|s| s.value()).unwrap_or(0),
        None => store
            .append(Event::pending(
                format!("u-{turn}"),
                symbio_core::EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                symbio_core::PRINCIPAL_MAIN,
            ))
            .expect("根事件入格")
            .value(),
    };
    let snapshot = store.range(Seq::new(0));
    write(
        store,
        &snapshot,
        decls,
        turn,
        anchor,
        symbio_core::PRINCIPAL_MAIN,
        &format!("v2t-test-a{turn}"),
    )
    .expect("任务事件入格");
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
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
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
    let ready = readyset(usize::MAX)
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
    let violations = check_all(&snapshot);
    assert!(
        violations.iter().any(|v| v.why.contains("依赖环")),
        "成环必须被断言抓到：{violations:?}"
    );
    let ready = readyset(usize::MAX)
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

    let violations = check_all(&store.range(Seq::new(0)));
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
        check_all(&snapshot).is_empty(),
        "一轮返工在界内（`MAX_REWORK_ROUNDS` = 3）：{:?}",
        check_all(&snapshot)
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
    let violations = check_all(&snapshot);
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

/// **压缩不缩小就绪集**（P6-② 的愈合点，[plan/14 §2.2](../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 推论 1）：
/// 任务开在压缩之前 ⇒ 调度段仍要念得出它。就绪集是**状态**口径
/// （「压缩前开的、还没做完」= 现在还没做完），不是「最近没被取代的那些事件」。
///
/// 存储层归档（被取代的前缀移进归档段）不改变 `Store::range` 的「完整史」语义
/// （[D-085](../../symbio-plugin-session/README.md#d-085-压缩取代的是消息列表不是轮次生命周期恢复锚读完整史归档刀口不动 "crates/symbio-plugin-session/src/README")；归档自身的判据在
/// `session::store::wal.test.rs`）。本用例因此只需一份「较早任务格 + 较晚快照」的完整史。
///
/// **反向自检**：把 `task_events::prompt_section` 的切片换成「从最新快照起自切窗口」
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

/// 开一个任务（载荷 `{ task_id, depends_on, goal }`）——生产写方见本模块 `write`。
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
        let view = readyset(1).apply(&snap, now, Budget::generous()).value;
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
    let view = readyset(1).apply(&snap, 1_000, Budget::generous()).value;
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
    let view = readyset(1).apply(&snap, 9_999, Budget::generous()).value;

    // 只持 `produce.artifact` 的工人主体：能产物，不能派活。
    let speaker = trivial_actor_spec("session/worker");
    assert!(
        ScheduleDecider
            .decide(&speaker, &view, &snap, PRINCIPAL_SCHEDULER, 9_999)
            .is_empty(),
        "缺 assign.work ⇒ 零事实（没有资格的调度不留痕）"
    );
}
