//! 任务表写方验收（S7 步 16–18，[04 §3.1 批⑨](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：
//! **清单声明 → 事实 → 两个消费方**的每一环都要看得见，且每条口径各配一个反向用例
//! （声明不变 ⇒ 不补事件；降级 ⇒ 不产事实；成环 ⇒ `invariants` 必须报出来）。

use super::{prompt_section, write};
use crate::symbio_core::facts::{TaskDeclaration, TaskItem, TaskStatus};
use crate::symbio_core::{
    check_all, readyset, Budget, Entity, Event, EventStore, Seq, Store, Verb,
    EVENT_SESSION_CONSOLIDATED, EVENT_TASK_ASSERTED, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
    EVENT_TASK_REWORK_CREATED,
};

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
fn snapshot_node(id: &str) -> crate::symbio_core::chat_node::ChatNode {
    use crate::symbio_core::chat_node::{MessageContent, NodeKind, NodeStatus};
    crate::symbio_core::chat_node::ChatNode {
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
    let snapshot = store.range(Seq::new(0));
    write(
        store,
        &snapshot,
        decls,
        turn,
        0,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
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
    assert_eq!(
        snapshot.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec![
            EVENT_TASK_OPENED,
            EVENT_TASK_PROGRESS,
            EVENT_TASK_OPENED,
            EVENT_TASK_OPENED,
            EVENT_TASK_ASSERTED,
        ],
        "t1 开 + 推进、t2 开（挂 t1）、t3 开 + 断言"
    );
    let opened = &snapshot[0];
    assert_eq!(
        opened
            .payload
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0)
    );
    assert_eq!(
        snapshot[2].payload.get("depends_on").cloned(),
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
    let ready = readyset()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value
        .ready;
    let ids: Vec<&str> = ready.iter().map(|t| t.task_id.as_str()).collect();
    assert_eq!(ids, vec!["t1", "t3"], "t2 依赖 t1（未终态）⇒ 不就绪");

    let section = prompt_section(&snapshot).expect("有就绪任务 ⇒ 有调度段");
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
    let ready = readyset()
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
    assert!(prompt_section(&[]).is_none(), "没有事实源 ⇒ 无段落");

    // 只有已终态的任务 ⇒ 就绪集空 ⇒ 同样无段落。
    let done = EventStore::new();
    write_all(
        &done,
        0,
        &[decl(vec![item("t1", "甲", &[], TaskStatus::Completed)])],
    );
    assert!(
        prompt_section(&done.range(Seq::new(0))).is_none(),
        "全终态 ⇒ 无候选 ⇒ 不给模型一段空调度"
    );
}

/// **压缩不缩小就绪集**（P6-② 的愈合点，[plan/14 §2.2](../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 推论 1）：
/// 任务开在压缩之前 ⇒ 调度段仍要念得出它。就绪集是**状态**口径
/// （「压缩前开的、还没做完」= 现在还没做完），不是「最近没被取代的那些事件」。
///
/// 存储层归档（被取代的前缀移进归档段）不改变 `Store::range` 的「完整史」语义
/// （[ADR-085](../../../../docs/010-adr/085.md "ADR-085")；归档自身的判据在
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
                crate::symbio_core::authz::PRINCIPAL_MAIN,
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

    let section = prompt_section(&store.range(Seq::new(0))).expect("压缩不缩小就绪集 ⇒ 仍有调度段");
    assert!(
        section.contains("t1"),
        "压缩前开的那条任务必须在候选集里：{section}"
    );
}
