//! `task.readyset` 的单测：**谁就绪、什么序**在这里算，契约在读方（本包的调度段 /
//! `session` 的统计列）。
//!
//! 与实现**同级**分文件（`readyset.rs` + 同级 `readyset.test.rs`）。本文件钉的是「读出来是哪几笔」：
//! 依赖闭合才算就绪、链式推进一次一笔、挂起排除与恢复（含 as-of 重放）、活租约排除与到期即回、
//! 返工开新节点不复活旧节点、`cap` 由读者套而不进视图、同一切片永远同一个集合（N1）。
//!
//! 这些判据搬出前住在 `crates/symbio-sdk/src/actors/mod.test.rs` 的「彩排：任务树与返工」
//! 一节（as-of 那一半在 `crates/symbio-plugin-session/src/transcript/inbox.test.rs`）。
//! `D-289` 把折叠搬到 owner 包，判据跟着走——**改的是住在哪个 crate，不是算式**：
//! 用例里的事件形状与搬出前逐条同构（开格带 `depends_on`、终态带溯源、租约带 `until_ms`）。
//!
//! ## 为什么这里的 `now` 是小得离谱的数
//!
//! `EventStore` 不兜底入库时刻（大量 as-of 用例靠 `ts = 0` 做断言，兜底会毁掉它们），
//! 所以 `now = 9_999` 在这个夹具里是「晚于一切已入格事件」的意思。租约那一组要的是
//! **到期前后各读一次**，于是 `until_ms = 1_000` 配 `now = 999 / 1_000`——测的是折叠认
//! 时间戳，不是认时钟。
//!
//! ## 为什么不复算读方
//!
//! `session` 那一侧的列不能在这里比对（`E-009` 禁兄弟插件依赖，测试也禁）：它按名字读
//! 的是本视图的**同一份**求值，所以「列 == 视图」由装配期只有一份算式保证，不靠两处比对。
//! 跨插件的读数一致性归 e2e（`t31` 调度容量、`t34` 就绪与仲裁、`t36` 租约到期重派），
//! 本文件只管算式本身。

use super::{ready_all, ready_view, readyset_value, ReadySetView};
use crate::decl::{
    EVENT_TASK_ASSERTED, EVENT_TASK_ASSIGNED, EVENT_TASK_HELD, EVENT_TASK_OPENED,
    EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED, READYSET_VIEW,
};
use symbio_plugin_api::{
    check_all, Entity, Event, EventStore, Seq, Store, Verb, PRINCIPAL_MAIN, PRINCIPAL_SCHEDULER,
};

/// 开一个任务（载荷 `{ task_id, depends_on, goal }`——与 `todo_write` 落格的形状同构）。
fn open_task(store: &EventStore, id: &str, depends_on: &[&str]) {
    store
        .append(
            Event::pending(
                format!("o-{id}"),
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                PRINCIPAL_MAIN,
            )
            .with_payload(
                serde_json::json!({ "task_id": id, "depends_on": depends_on, "goal": "goal" }),
            ),
        )
        .expect("开任务必入库");
}

/// 终态：验证通过（`Task × Asserted`，断言类事件必须带溯源）。
fn assert_task(store: &EventStore, id: &str) {
    store
        .append(
            Event::pending(
                format!("done-{id}"),
                EVENT_TASK_ASSERTED,
                Entity::Task,
                Verb::Asserted,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": id })),
        )
        .expect("终态必入库");
}

/// 推进一格（在就绪判定里它同时是**解除挂起**）。
fn progress_task(store: &EventStore, id: &str) {
    store
        .append(
            Event::pending(
                format!("p-{id}"),
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                PRINCIPAL_MAIN,
            )
            .with_payload(serde_json::json!({ "task_id": id })),
        )
        .expect("推进必入库");
}

/// 挂起一格（插话抢占留下的痕迹）。`ts` 由调用方给：折叠认时间戳，不认时钟。
fn hold_task(store: &EventStore, id: &str, ts: i64) {
    store
        .append(
            Event::pending(
                format!("h-{id}"),
                EVENT_TASK_HELD,
                Entity::Task,
                Verb::Held,
                PRINCIPAL_MAIN,
            )
            .with_ts(ts)
            .with_payload(serde_json::json!({ "task_id": id, "reason": "interrupted" })),
        )
        .expect("挂起必入库");
}

/// 落一条租约。`until_ms` 是**到期时刻**；交 `None` 就是「没写这一格」的那条反向。
fn lease_task(store: &EventStore, id: &str, until_ms: Option<i64>) {
    let mut payload = serde_json::json!({ "task_id": id, "owner": PRINCIPAL_SCHEDULER });
    if let Some(ms) = until_ms {
        payload["until_ms"] = serde_json::json!(ms);
    }
    store
        .append(
            Event::pending(
                format!("as-{id}"),
                EVENT_TASK_ASSIGNED,
                Entity::Task,
                Verb::Asserted,
                PRINCIPAL_SCHEDULER,
            )
            .with_produced_by(0)
            .with_payload(payload),
        )
        .expect("租约入格");
}

/// 一次返工：新增节点、指向被取代的那一个（append-only，旧节点不改）。
fn rework_node(store: &EventStore, id: &str, replaces: &str, round: u64) {
    store
        .append(
            Event::pending(
                format!("rw-{id}"),
                EVENT_TASK_REWORK_CREATED,
                Entity::Task,
                Verb::Asserted,
                PRINCIPAL_MAIN,
            )
            .with_produced_by(0)
            .with_payload(
                serde_json::json!({ "task_id": id, "replaces": replaces, "round": round }),
            ),
        )
        .expect("返工入格");
}

fn ids(ready: &[super::ReadyTask]) -> Vec<&str> {
    ready.iter().map(|t| t.task_id.as_str()).collect()
}

/// 就绪集 = 非终态 ∧ 依赖全部终态；终态之后链式推进一格；同一切片双跑一致（N1）。
#[test]
fn the_ready_set_is_dependency_closed_and_advances_one_node_at_a_time() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    open_task(&store, "b", &["a"]);
    open_task(&store, "c", &["b"]);
    let snapshot = store.range(Seq::new(0));

    assert_eq!(ids(&ready_all(&snapshot, 9_999)), ["a"], "只有 a 就绪");

    // a 终态 → 就绪集挪到 b：调度器零内部状态，下一笔由同一份事实算出。
    assert_task(&store, "a");
    let snapshot = store.range(Seq::new(0));
    assert_eq!(ids(&ready_all(&snapshot, 9_999)), ["b"]);

    // N1：双跑逐字节一致——视图值也一样，因为它是同一份 `ready_all` 的序列化。
    let again = ready_all(&snapshot, 9_999);
    assert_eq!(ready_all(&snapshot, 9_999), again);
    assert_eq!(
        readyset_value(&snapshot, 9_999),
        readyset_value(&snapshot, 9_999),
        "同切片同 as-of ⇒ 两个读者读到的必然是同一份"
    );
}

/// `cap` 是读者的预算，不是折叠的一格：视图交无界候选集，`ready_view` 那一层才截断。
///
/// 搬出 core 之前这件事只能写在文档里（投影的形参表收得下 `cap`）；今天声明视图的求值口
/// 只有切片与时间戳，**容量根本没有进来的通道**——所以「视图里不含 `cap`」这一条
/// 是形状给的保证，不是约定。
#[test]
fn the_capacity_is_the_readers_budget_not_a_fold_parameter() {
    let store = EventStore::new();
    for id in ["a", "b", "c", "d"] {
        open_task(&store, id, &[]);
    }
    let snapshot = store.range(Seq::new(0));

    // 折叠本身：四笔全交，按入队序。
    assert_eq!(ids(&ready_all(&snapshot, 9_999)), ["a", "b", "c", "d"]);
    assert_eq!(
        readyset_value(&snapshot, 9_999)["ready"]
            .as_array()
            .map(|a| a.len()),
        Some(4),
        "对外的视图不设容量：截几笔是读者的事"
    );

    // 读者套上预算：`cap = 1` 退化为串行 FIFO，先开的先派。
    let one = ready_view(&snapshot, 9_999, 1);
    assert_eq!(ids(&one.ready), ["a"]);
    assert_eq!(one.cap, 1);
    assert_eq!(
        ready_view(&snapshot, 9_999, 4).ready.len(),
        4,
        "同一套判据放开四条"
    );

    // `cap = 0` 与 1 同义：容量为零的调度器不是「更保守」，是「永不动」。
    // 视图里的 `cap` 因此是**生效**容量——报回原值会让「配了 0」与「配了 1」读起来不一样。
    let zero = ready_view(&snapshot, 9_999, 0);
    assert_eq!(zero.cap, 1);
    assert_eq!(ids(&zero.ready), ["a"]);

    // 反向：截断不是判据变了——同一份切片，`cap` 只改长度，不改成员与顺序：
    // 截两笔的前缀就是截一笔的全部。
    let two = ready_view(&snapshot, 9_999, 2);
    assert_eq!(
        &two.ready[..one.ready.len()],
        one.ready.as_slice(),
        "截几笔不改变先派谁"
    );
    assert_eq!(two.cap, 2);
    assert_eq!(ids(&two.ready), ["a", "b"]);
}

/// 活租约让任务离开候选集；到点即回（判死的事实由调度者补落，折叠只认时间戳）。
#[test]
fn a_live_lease_holds_a_task_out_and_expiry_returns_it() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    lease_task(&store, "a", Some(1_000));
    let snapshot = store.range(Seq::new(0));

    assert!(
        ready_all(&snapshot, 999).is_empty(),
        "租约未到期 ⇒ 被派出去的活不再进候选（不重复派）"
    );
    assert_eq!(
        ready_view(&snapshot, 1_000, 1).ready.len(),
        1,
        "到点即回候选集"
    );
    // 终态之后租约没有意义：那笔活已经交出去了，不再占着候选集。
    assert_task(&store, "a");
    let snapshot = store.range(Seq::new(0));
    assert!(
        ready_all(&snapshot, 500).is_empty(),
        "已终态 ⇒ 既不就绪也不该被算成「被租约占住」"
    );
}

/// 缺 `until_ms` 的租约按「没有租约」处理，而不是「永久持有」。
///
/// 反向用例：写方漏一格时系统的失效方向是**重复派一次**，不是永远不派。
#[test]
fn a_lease_without_until_ms_is_no_lease_at_all() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    lease_task(&store, "a", None);
    let snapshot = store.range(Seq::new(0));
    assert_eq!(
        ready_view(&snapshot, 9_999, 1).ready.len(),
        1,
        "无期限租约不成立：宁可重复派，不要永远不派"
    );
}

/// 挂起排除（DE-037 §5 的强制点）与 §6 验收 2、3：挂起期间不得被二次调度；
/// 以挂起锚 as-of 重放与挂起前逐字节一致（恢复零状态迁移——折叠是纯函数）；
/// 一条 `task.progressed` 就让任务重新就绪。
#[test]
fn a_suspended_task_leaves_the_ready_set_and_progress_restores_it() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    let snapshot = store.range(Seq::new(0));
    let before = ready_all(&snapshot, 9_999);
    assert_eq!(ids(&before), ["a"], "前置：a 就绪");

    hold_task(&store, "a", 5_000);
    let snapshot = store.range(Seq::new(0));
    assert!(
        ready_all(&snapshot, 9_999).is_empty(),
        "挂起 ⇒ 离开候选集（否则重复执行）"
    );
    // 验收 3：以挂起锚（`now` 早于那条 held 的 ts）重放 ⇒ 与挂起前逐字节相等。
    // 「恢复不带状态迁移」的可执行形式就是这个等式：同一份事实源，给同一个 T 就得同一个视图。
    assert_eq!(
        ready_all(&snapshot, 1_000),
        before,
        "as-of=锚 的重放与挂起前一致（DE-037 §6 验收 3）"
    );

    progress_task(&store, "a");
    let snapshot = store.range(Seq::new(0));
    let nodes = ready_all(&snapshot, 9_999);
    assert_eq!(
        ids(&nodes),
        ["a"],
        "恢复也是一条事件：折叠不持有「现在挂没挂」这个状态"
    );
    assert_eq!(
        nodes, before,
        "恢复后的候选集与挂起前逐字节相同：没有一处状态被搬动"
    );
}

/// 返工开**新节点**，不复活旧的：旧节点仍未终态，所以它照旧在候选集里（Log 永不改）。
#[test]
fn rework_opens_a_new_node_without_reviving_the_old_one() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    open_task(&store, "b", &["a"]);
    rework_node(&store, "a-r1", "a", 1);
    open_task(&store, "a-r1", &[]);
    let snapshot = store.range(Seq::new(0));
    let nodes = ready_all(&snapshot, 9_999);
    let ready = ids(&nodes);
    assert!(ready.contains(&"a-r1"), "返工节点就绪：{ready:?}");
    assert!(
        ready.contains(&"a"),
        "旧节点仍未终态——被取代靠新节点表达，不是回滚：{ready:?}"
    );

    // 新节点过验也不解锁 b：b 依赖的是 a，不是 a-r1（依赖不会跟着返工改写）。
    assert_task(&store, "a-r1");
    let snapshot = store.range(Seq::new(0));
    let nodes = ready_all(&snapshot, 9_999);
    let ready = ids(&nodes);
    assert!(
        !ready.contains(&"b"),
        "b 的依赖是 a（未终态），依旧不就绪：{ready:?}"
    );
}

/// DE-033 §6 验收 3 的折叠半边：N 步链任意时刻恰一个就绪，全链终态后为空。
#[test]
fn an_n_step_chain_advances_exactly_one_task_at_a_time() {
    let store = EventStore::new();
    let n = 5;
    for i in 0..n {
        let deps: Vec<String> = if i == 0 {
            vec![]
        } else {
            vec![format!("t{}", i - 1)]
        };
        let deps: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        open_task(&store, &format!("t{i}"), &deps);
    }
    for i in 0..n {
        let snapshot = store.range(Seq::new(0));
        let ready = ready_all(&snapshot, 9_999);
        assert_eq!(ready.len(), 1, "链式调度任意时刻恰一个就绪：{ready:?}");
        assert_eq!(ready[0].task_id, format!("t{i}"));
        progress_task(&store, &format!("t{i}"));
        assert_task(&store, &format!("t{i}"));
    }
    let snapshot = store.range(Seq::new(0));
    assert!(
        ready_all(&snapshot, 9_999).is_empty(),
        "全链终态 ⇒ 候选集诚实为空"
    );
    // 断言面（C14 无环 + 溯源）也在同一份切片上跑一遍：折叠说「跑完了」，
    // 断言说「这一路没有违规」——两条都在，验收 3 才算闭合。
    let violations = check_all(symbio_plugin_api::vocab(), &snapshot);
    assert!(violations.is_empty(), "全链跑完不留违规：{violations:?}");
}

/// 声明的视图在冻结声明表里按名字读得出，且读出的就是折叠那一份（**不含** `cap`）。
///
/// 这一条钉的是「owner 交的是同一张表」：`session` 的统计列与本包的调度段读的是同一个
/// `eval`，容量各套各的——所以两侧的前缀必然同源。视图名若改名、或这条 `ViewDecl`
/// 从 `decl.rs::VIEWS` 里掉出去，这条先红（报表登记的 `views` 指向同一个名字，
/// 由 `decl.test.rs` 的登记用例守）。
#[test]
fn the_declared_view_is_readable_by_name_and_carries_no_capacity() {
    let store = EventStore::new();
    open_task(&store, "a", &[]);
    open_task(&store, "b", &["a"]);
    let snapshot = store.range(Seq::new(0));

    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    let view = frozen.view(READYSET_VIEW).expect("视图应在冻结声明表里");
    let by_name = (view.eval)(&snapshot, i64::MAX);
    assert_eq!(
        by_name,
        readyset_value(&snapshot, i64::MAX),
        "声明视图 = 折叠本身，不在读口再算一遍"
    );
    assert!(
        by_name.get("cap").is_none(),
        "容量不进对外那一份：{by_name}"
    );

    // 反向：`ready` 之外的键一个也不交（形状归 owner，读方不必分诊）。
    let keys: Vec<&str> = by_name
        .as_object()
        .expect("视图值是对象")
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(keys, ["ready"]);
}

/// 读者的空形状与生效容量：`cap` 落进列的是**截断之后**那一份（`D-289` 的另一半判据）。
///
/// 本包的调度段（[`ReadySetView`]）交 `ready` + `cap` 两键；对外的视图只交 `ready`。
/// 两者必须能同时成立——否则「为什么只给这几笔」这个信号在出口与在候选集里长得不一样。
#[test]
fn the_reader_shapes_the_capacity_while_the_view_stays_unbounded() {
    let store = EventStore::new();
    for id in ["a", "b", "c"] {
        open_task(&store, id, &[]);
    }
    let snapshot = store.range(Seq::new(0));

    let view: ReadySetView = ready_view(&snapshot, i64::MAX, 2);
    assert_eq!(view.ready.len(), 2);
    assert_eq!(view.cap, 2);

    let declared = readyset_value(&snapshot, i64::MAX);
    assert_eq!(
        declared["ready"].as_array().map(|a| a.len()),
        Some(3),
        "同一切片：视图无界、读者有预算"
    );
}
