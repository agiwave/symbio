//! `session/stats` 读数口的判据（[plan/12 §4](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
//!
//! 1. **复算**——出口的读数与直接 `apply` 逐字相等。同一切片、同一个 as-of：
//!    出口只能调已有的投影，不许另写一份统计（否则两套口径会静默漂移）。
//! 2. **反向**——从事实源里删掉一条事件，对应那一列必须跟着变（证明它真的在算，
//!    不是常数）。
//! 3. **纯读**——没有事实源时是「有据的零」，且**不得**为了读而创建文件。
//! 4. **读侧闸**——声明读方身份才判可见域：属主全量、非属主全零（批⑥）。

use super::{fleet_reading, read};
use crate::plugins::session::store::EventWalStore;
use crate::symbio_core::{
    calibration, check_all, checkpoint, cost_ledger, fallback_rate, slo_report, transcript, Budget,
    Entity, Event, EventEnvelope as _, Seq, Store, Verb, EVENT_ARTIFACT_ADDED,
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED,
    EVENT_SESSION_CONSOLIDATED, EVENT_TASK_OPENED, EVENT_USER_MESSAGE, SKILL_TAG,
};
use std::path::{Path, PathBuf};

/// 一个会话的事实**值**（生产侧由 `SessionStore::snapshot` 交回）。
fn facts_of(wal: &Path) -> Option<Vec<Event>> {
    if !wal.is_file() {
        return None;
    }
    let store = EventWalStore::open_readonly(wal).ok()?;
    Some(store.range(Seq::new(0)))
}

/// 部署级快照（测试侧按生产布局自己深遍；生产侧这段在存储层）。
fn fleet_of(root: &Path) -> Vec<Vec<Event>> {
    super::super::paths::session_wals_deep(root)
        .into_iter()
        .filter_map(|wal| EventWalStore::open_readonly(&wal).ok())
        .map(|store| store.range(Seq::new(0)))
        .collect()
}

/// 读数（**测试专用窄口**）：把路径翻成值后调 [`read`]。
fn read_path(
    session_id: &str,
    wal: &Path,
    viewer: Option<&str>,
    principal: &str,
    root: &Path,
) -> Result<super::SessionStats, crate::symbio_core::PluginError> {
    read(
        session_id,
        facts_of(wal),
        || fleet_of(root),
        viewer,
        principal,
        false,
        crate::symbio_core::TASK_CAP,
    )
}

/// 单会话读数：`storage_root` 固定成 WAL 所在目录本身。
///
/// 于是每个用例都只有一个会话 ⇒ 聚合列读到的就是「一个 WAL」的形状，可与
/// per-session 各列对照。P2 的多会话判据另建目录、不走这个窄口。
///
/// 为什么包一层而不是给 21 处调用都加参数：测试要盯的是**各列口径**，
/// 聚合列的扫描范围由它自己那条判据钉。两件事混在一个入口上，改聚合范围
/// 就要动 21 处。
fn read_at(wal: &Path, principal: &str) -> super::SessionStats {
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    read_path("s1", wal, None, principal, &root).expect("读数")
}

/// 播种一个事实源：两轮（一轮成功 final、一轮兜底 fallback），共 4 格。
///
/// 两轮都声明 `tier = deep`——分母 / 分子 / 样本数因此都落在同一行上，
/// 断言不必先做档位归并。
fn seed(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();

    // 轮 0：开轮 → final（实测 1200ms，成功轮才计入时延样本）
    let u0 = Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好", "tier": "deep", "turn_ref": "u-0" }));
    let s0 = store.append(u0).unwrap().value();
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "session/main",
            )
            .with_produced_by(s0)
            .with_cost_ms(1200)
            .with_payload(serde_json::json!({ "text": "答" })),
        )
        .unwrap();

    // 轮 1：开轮 → 兜底（失败轮的耗时不算时延样本，只进兜底分子）
    let u1 = Event::pending(
        "u1",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        1,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "再问", "tier": "deep", "turn_ref": "u-1" }));
    let s1 = store.append(u1).unwrap().value();
    store
        .append(
            Event::pending(
                "fb1",
                EVENT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                1,
                "session/main",
            )
            .with_produced_by(s1)
            .with_cost_ms(3000)
            .with_payload(serde_json::json!({ "reason": "fallback", "detail": "模型超时" })),
        )
        .unwrap();

    wal
}

/// 按 `event_id` 删掉一行（反向用例的手术刀）。
fn drop_line(wal: &Path, event_id: &str) {
    let raw = std::fs::read_to_string(wal).expect("读 WAL");
    let kept: Vec<&str> = raw
        .lines()
        .filter(|line| {
            let keep = serde_json::from_str::<serde_json::Value>(line)
                .map(|v| v.get("event_id").and_then(|x| x.as_str()) != Some(event_id))
                .unwrap_or(true);
            keep
        })
        .collect();
    std::fs::write(wal, format!("{}\n", kept.join("\n"))).expect("回写 WAL");
}

// ── 实例级聚合列（P2）──────────────────────────────────────────────────────
//
// ## 为什么单独一组用例
//
// 其余各列的口径是「一个会话、一份切片、一套投影」；聚合列的口径是「**实例下所有
// 会话**、多份切片、合并后再出数」。它多出来的自由度正是它独有的失效形态：
// 少扫一个会话、把 archive 扫进来、一个坏 WAL 拖垮整列——三条都**不报错**，
// 只是数字悄悄不对（样本少几个、目录多几个）。

/// 造一个有 N 个会话的实例根，每个会话一格 final（同档位 `deep`）。
///
/// 返回 `(实例根, 各会话的 WAL 路径)`。**会话目录 = 存储根的直接子目录**
/// （`paths::session_dir` 的形状），故意按真布局造——判据要盯的正是
/// 「扫描范围对不对」，用假布局就照不出范围错。
fn seed_instance(tag: &str, n: usize) -> (PathBuf, Vec<PathBuf>) {
    let root = std::env::temp_dir().join(format!("symbio-fleet-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("实例根");
    let mut wals = Vec::new();
    for i in 0..n {
        let dir = root.join(format!("session-{i}"));
        std::fs::create_dir_all(&dir).expect("会话目录");
        let wal = dir.join(super::super::paths::V2_WAL_FILE);
        let store = EventWalStore::open(&wal).expect("WAL");
        // 每个会话一轮：final 实测各不相同 ⇒ 合并后样本数与分位数都可判
        let u = store
            .append(
                Event::pending(
                    format!("u{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap()
            .value();
        store
            .append(
                Event::pending(
                    format!("f{i}"),
                    EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "session/main",
                )
                .with_produced_by(u)
                .with_cost_ms(100 * (i as u64 + 1)),
            )
            .unwrap();
        wals.push(wal);
    }
    (root, wals)
}

/// **正向**：三个会话 ⇒ `wals = 3`、样本 3 个、时延取全量口径。
///
/// 这条是 P2 的出口判据本身——它红的方式有两种，都是静默的：扫不到（`wals` 少）、
/// 扫错范围（`wals` 多）。两者都不抛错，所以必须有一个断言直接盯数字。
#[test]
fn fleet_aggregates_every_session_in_the_instance() {
    let (root, wals) = seed_instance("agg", 3);
    let got = read_path(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");

    assert_eq!(
        got.fleet["wals"], 3,
        "实例下三个会话的 WAL 全部读进来（少一个就是静默漏扫）"
    );
    assert_eq!(got.fleet["sessions"], 3, "目录里有 WAL 的会话数");
    assert_eq!(
        got.fleet["by_tier"]["latency"][0]["samples"], 3,
        "三个会话各一格 final ⇒ 合并后 3 个样本（并集，不是覆盖）"
    );
    // 样本是 100/200/300 ⇒ P50 = 200（全量口径）。若只扫到本会话，P50 会是 100。
    assert_eq!(
        got.fleet["by_tier"]["latency"][0]["p50"], 200,
        "分位数按合并后的全量样本算，不是任一会话自己的"
    );
    // 兜底分母同样是全量口径（三格开轮，零兜底）
    assert_eq!(got.fleet["turns"], 3);
    assert_eq!(got.fleet["fallbacks"], 0);
}

/// **反向（范围）**：只走一层——`tool_archives` 里的同名文件不算事实源。
///
/// 递归扫会把 archive 读进来，`wals` 于是虚高、样本混入归档。这条把假 WAL
/// 塞进 **两层深**的子目录，`wals` 必须不动。
#[test]
fn fleet_scan_does_not_descend_into_session_subdirectories() {
    let (root, wals) = seed_instance("shallow", 1);
    // 会话子目录里再放一个同名文件（形如 `<root>/session-0/tool_archives/v2-events.wal`）
    let nested = wals[0].parent().unwrap().join("tool_archives");
    std::fs::create_dir_all(&nested).expect("子目录");
    std::fs::copy(&wals[0], nested.join(super::super::paths::V2_WAL_FILE)).expect("放假 WAL");

    let got = read_path(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet["wals"], 1,
        "只扫一层：子目录里的同名文件不是事实源（递归会虚高）"
    );
}

/// **反向（韧性）**：一个**根本不是 WAL** 的路径不进清单、不拖垮整列。
///
/// 撕裂尾行被写方修复是正常状态，`open_readonly` 对「读得出一部分」是会成功的
/// （实测写坏内容仍开得出、只是 0 条事件）。枚举的判据是**存在性**（是文件）：
/// 把 WAL 换成**目录**，连清单都进不了——跳过韧性由存储层的深遍负责
/// （本用例只钉出口读到的值形状）。
#[test]
fn fleet_survives_a_session_whose_wal_cannot_be_opened() {
    let (root, wals) = seed_instance("torn", 2);
    // 把第二个会话的 WAL 换成一个**目录**：路径存在但不是文件 ⇒ 不算事实源。
    let _ = std::fs::remove_file(&wals[1]);
    std::fs::create_dir_all(&wals[1]).expect("把 WAL 换成目录");

    let got = read_path(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(got.fleet["wals"], 1, "目录不是事实源：存在性枚举根本不认它");
    assert_eq!(
        got.fleet["sessions"], 1,
        "sessions = 清单数（是文件的路径），不是「路径存在」数"
    );
    assert_eq!(got.fleet["by_tier"]["latency"][0]["samples"], 1);
}

/// **聚合吃值**：交几份快照就合并几份，不碰路径、不自行枚举。
///
/// 「哪些事实源打得开」由存储层判（`deep_snapshots` 跳过并出声）；本列只对
/// 交到手里的值套投影——两半不要混在一个函数里（[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）。
#[test]
fn fleet_aggregates_the_snapshots_it_is_given() {
    let (root, _wals) = seed_instance("values", 2);
    let facts = fleet_of(&root);
    assert_eq!(facts.len(), 2, "存储层深遍交回两个会话的值");

    let got = fleet_reading(&facts);
    assert_eq!(got["wals"], 2);
    assert_eq!(got["sessions"], 2, "与 wals 同值：都是交到手里的快照数");
    assert_eq!(
        got["by_tier"]["latency"][0]["samples"], 2,
        "两份快照各一格 final ⇒ 样本并集是 2"
    );
}

/// **读侧闸**：不给读的人 ⇒ 聚合列是空形状，且**不扫**。
///
/// 扫遍实例再报「不给你看」是白花代价；结论与空形状相同，故直接走空。
#[test]
fn fleet_is_empty_when_the_reader_may_not_read() {
    let (root, wals) = seed_instance("gate", 2);
    let got = read_path(
        "session-0",
        &wals[0],
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.fleet,
        serde_json::json!({
            "wals": 0, "sessions": 0, "turns": 0, "fallbacks": 0,
            "by_tier": { "latency": [], "fallback": [] },
            "axes": {
                "conation": { "expressed": 0, "escalated": 0, "adoption_rate": 0.0 },
                "span": { "sessions": 0, "median_ms": null },
                "autonomy": { "closed": 0, "unattended": 0, "unattended_rate": 0.0 },
            },
        }),
        "不给你读 ⇒ 空形状（与满读数同键：三族给有据的零 / 空，不缺列），不扫也不报数"
    );
}

// ── 外部效标两列（02 §3.1：主动性 / 时域跨度）────────────────────────────────
//
// ## 为什么单独一组用例
//
// 这两列与四列同族（同切片、同 as-of、同一条读侧闸），判据因此复用
// 「改输入会变红」的手术刀（`drop_line`）：把一条欲事件删掉 ⇒ 分子 / 分母跟着动；
// 把末事件删掉 ⇒ 跨度缩短。**读数口是这两列的全部意义**——没有反向用例的
// 「效标」只是表格里的一行字。

/// 主动性列：分子（已升格）/ 分母（已表达）同出一列；`may_read = false` 时
/// 走四列形态（空切片 ⇒ 空视图），不报零值条目。
#[test]
fn conation_column_recomputes_and_moves_with_the_wal() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-conation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();

    // 链路：触发 → 欲 A（升格：task.opened 的 produced_by 指回它）/ 欲 B（未升格）。
    let trigger = store
        .append(
            Event::pending(
                "sys-0",
                crate::symbio_core::EVENT_SYSTEM_TRIGGERED,
                Entity::System,
                Verb::Opened,
                0,
                crate::symbio_core::PRINCIPAL_AUTONOMOUS,
            )
            .with_ts(10)
            .with_payload(serde_json::json!({ "kind": "scheduled" })),
        )
        .unwrap()
        .value();
    let want_a = store
        .append(
            Event::pending(
                "want-a",
                crate::symbio_core::EVENT_CONATION_EXPRESSED,
                Entity::Conation,
                Verb::Opened,
                0,
                crate::symbio_core::PRINCIPAL_AUTONOMOUS,
            )
            .with_ts(20)
            .with_produced_by(trigger)
            .with_payload(serde_json::json!({ "goal": "目标 A" })),
        )
        .unwrap()
        .value();
    store
        .append(
            Event::pending(
                "want-b",
                crate::symbio_core::EVENT_CONATION_EXPRESSED,
                Entity::Conation,
                Verb::Opened,
                0,
                crate::symbio_core::PRINCIPAL_AUTONOMOUS,
            )
            .with_ts(30)
            .with_produced_by(trigger)
            .with_payload(serde_json::json!({ "goal": "目标 B" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "task-a",
                crate::symbio_core::EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                0,
                crate::symbio_core::PRINCIPAL_AUTONOMOUS,
            )
            .with_ts(40)
            .with_produced_by(want_a)
            .with_payload(serde_json::json!({ "task_id": "t-a" })),
        )
        .unwrap();

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let intents = got
        .conation
        .get("intents")
        .and_then(|v| v.as_array())
        .expect("intents 列");
    assert_eq!(intents.len(), 2, "分母 = 全部已表达");
    assert_eq!(
        intents
            .iter()
            .filter(|i| i.get("task_id").is_some_and(|t| !t.is_null()))
            .count(),
        1,
        "分子 = 已升格"
    );

    // 手术刀：删掉升格事件 ⇒ 分子归零（率跟着动，不是死数字）
    drop_line(&wal, "task-a");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let intents = after
        .conation
        .get("intents")
        .and_then(|v| v.as_array())
        .expect("intents 列");
    assert!(
        intents
            .iter()
            .all(|i| i.get("task_id").is_none_or(|t| t.is_null())),
        "删掉 task.opened ⇒ 没有任何欲算已升格"
    );

    // 读侧闸：非属主 ⇒ 四列形态的空（不是零值条目）
    let gated = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &dir,
    )
    .expect("读数");
    assert_eq!(
        gated.conation,
        serde_json::json!({ "intents": [], "adoption_rate": 0.0 }),
        "空切片 ⇒ 流为空且率是有据的零"
    );
}

/// 时域跨度列：跨度 = 末事件 ts − 首事件 ts；删末事件 ⇒ 跨度缩短（读数口会动）。
#[test]
fn span_column_recomputes_and_moves_with_the_wal() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-span-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();
    for (n, (id, ts)) in [("e0", 1_000i64), ("e1", 1_500), ("e2", 4_200)]
        .into_iter()
        .enumerate()
    {
        store
            .append(
                Event::pending(
                    id,
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    // 三条样例事件各占一轮：同一轮开两次会被写侧闸门拒掉（I1），
                    // 而本测试要的是「三条事件的时间跨度」，不是三轮业务语义。
                    n as u64,
                    "user",
                )
                .with_ts(ts)
                .with_payload(serde_json::json!({ "text": "问" })),
            )
            .unwrap();
    }

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(got.span.get("events").and_then(|v| v.as_u64()), Some(3));
    assert_eq!(
        got.span.get("span_ms").and_then(|v| v.as_u64()),
        Some(3_200)
    );

    // 手术刀：删掉末事件 ⇒ 跨度从 3200 缩到 500（读数跟着输入走）
    drop_line(&wal, "e2");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(
        after.span.get("span_ms").and_then(|v| v.as_u64()),
        Some(500)
    );
}

/// 实例级 axes 两族：跨会话合并（分子分母可加 / 中位数只收有跨度的会话）。
#[test]
fn fleet_axes_aggregate_across_sessions() {
    let (root, wals) = seed_instance("axes", 2);
    // 第二个会话补一条更晚的事件 ⇒ 跨度更长；两会话都只有 2 事件 ⇒ 都进中位数。
    {
        let store = EventWalStore::open(&wals[1]).unwrap();
        store
            .append(
                Event::pending(
                    "late",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    1,
                    "user",
                )
                .with_ts(9_000)
                .with_payload(serde_json::json!({ "text": "再问", "tier": "deep" })),
            )
            .unwrap();
    }
    let got = read_path(
        "session-0",
        &wals[0],
        None,
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    let axes = got.fleet.get("axes").expect("axes 族");
    assert_eq!(
        axes["conation"]["expressed"].as_u64(),
        Some(0),
        "实例里没有欲 ⇒ 有据的零"
    );
    assert_eq!(
        axes["span"]["sessions"].as_u64(),
        Some(2),
        "两会话都有 ≥2 事件 ⇒ 都进分母"
    );
    assert!(
        axes["span"]["median_ms"].is_u64(),
        "中位数有数（不是 null）"
    );
    assert_eq!(
        axes["autonomy"]["closed"].as_u64(),
        Some(0),
        "没有闭环任务 ⇒ 有据的零"
    );
    assert_eq!(axes["autonomy"]["unattended_rate"].as_f64(), Some(0.0));
}

/// 自主程度列：闭环且未挂起 / 闭环；删掉那条 `task.held` ⇒ 率从 0.5 升到 1.0
/// （读数口会动，不是表格里的一行字）。
#[test]
fn autonomy_column_recomputes_and_moves_with_the_wal() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-autonomy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();
    let task_ev = |id: &str, kind: &str, verb: Verb, task: &str, ts: i64| {
        Event::pending(
            id,
            kind,
            Entity::Task,
            verb,
            0,
            crate::symbio_core::PRINCIPAL_AUTONOMOUS,
        )
        .with_ts(ts)
        // 任务断言是 I2 的声明类事件：没有上游就没有可撤销的溯源。
        .with_produced_by(0)
        .with_payload(serde_json::json!({ "task_id": task }))
    };
    // t1 全程自主闭环；t2 被挂起过再闭环 ⇒ 不进分子
    store
        .append(task_ev(
            "h-2",
            crate::symbio_core::EVENT_TASK_HELD,
            Verb::Held,
            "t2",
            100,
        ))
        .unwrap();
    store
        .append(task_ev(
            "a-1",
            crate::symbio_core::EVENT_TASK_ASSERTED,
            Verb::Asserted,
            "t1",
            200,
        ))
        .unwrap();
    store
        .append(task_ev(
            "a-2",
            crate::symbio_core::EVENT_TASK_ASSERTED,
            Verb::Asserted,
            "t2",
            300,
        ))
        .unwrap();

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(got.autonomy.get("closed").and_then(|v| v.as_u64()), Some(2));
    assert_eq!(
        got.autonomy.get("unattended").and_then(|v| v.as_u64()),
        Some(1)
    );

    // 手术刀：删掉挂起事件 ⇒ t2 也算全程自主（率从 0.5 升到 1.0）
    drop_line(&wal, "h-2");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(
        after.autonomy.get("unattended").and_then(|v| v.as_u64()),
        Some(2),
        "删掉 task.held ⇒ 干预事实消失，分子跟着动"
    );
}

// ── 读数面板（P4：后端下发描述，前端零业务知识）────────────────────────────
//
// ## 这个机制唯一的失效形态
//
// 面板里的每个数都是从**同一份 SessionStats 的其它字段**搬过来的（`panel_of` 的签名
// 收 `&SessionStats`）。若哪天有人在 `panel_of` 里「顺手重算一遍」，两份口径就能
// 各自漂移，而**两个数单独看都对**——只有并排比才看得出来。所以这里的判据不验
// 「面板长什么样」，验「面板里的数 = 列里的数」。

/// 面板里的每个数都必须能在列里对上（同源，不是「看起来差不多」）。
#[test]
fn panel_matches_the_columns_it_describes() {
    let wal = seed("panel");
    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let panel = &got.panel;

    // 分区固定五段：时延兜底 / 事实源 / 不变量 / 技能 / 承诺
    assert_eq!(panel.sections.len(), 5, "分区固定：{:?}", panel.sections);
    assert_eq!(panel.subtitle.as_deref(), Some("s1"), "副标题带会话 id");

    // ① 时延段：一行对应一个档位，值与列逐字对上
    let perf = &panel.sections[0];
    assert_eq!(perf.rows.len(), got.tiers.len(), "一行一档，不多不少");
    for (row, col) in perf.rows.iter().zip(&got.tiers) {
        assert!(
            row.value.contains(&format!("{} 轮", col.turns)),
            "轮数要对上：{} vs {}",
            row.value,
            col.turns
        );
        assert!(
            row.value.contains(&format!("{:.0}%", col.rate * 100.0)),
            "兜底率要对上（0.* → 取整百分比）"
        );
        assert!(
            row.hint
                .as_deref()
                .unwrap_or("")
                .contains(&format!("P99 {}ms", col.p99)),
            "P99 要对上：{:?}",
            row.hint
        );
    }

    // ② 事实源段：条数 = 断点列的 event_count
    let cont = &panel.sections[1];
    let facts_row = cont
        .rows
        .iter()
        .find(|r| r.label == "已记录事实")
        .expect("有条数行");
    assert!(
        facts_row.value.contains(
            &got.checkpoint["event_count"]
                .as_u64()
                .unwrap_or(0)
                .to_string()
        ),
        "事实条数 = 断点列：{} vs {}",
        facts_row.value,
        got.checkpoint["event_count"]
    );
    let has_row = cont
        .rows
        .iter()
        .find(|r| r.label == "事实源在不在")
        .expect("有在不在行");
    assert_eq!(has_row.value, "在", "seed 造过事实源");
    assert_eq!(has_row.tone, crate::symbio_core::ReadingTone::Normal);

    // ③ 不变量段：seed 的 4 格干净 ⇒ 0 条违规、平常语气
    let inv = &panel.sections[2];
    assert_eq!(inv.rows.len(), 1);
    assert_eq!(inv.rows[0].value, "0 条", "干净的事实源 ⇒ 0 违规");
    assert_eq!(inv.rows[0].tone, crate::symbio_core::ReadingTone::Normal);
    // 反过来：seed 的兜底率是 1/2 = 50%，远超 5% 阈值 ⇒ 时延段**本该**是 warn。
    // 这一条与下面「有违规 ⇒ warn」合起来覆盖两个方向：该标色的标、不该标的不标。
    assert_eq!(
        perf.rows[0].tone,
        crate::symbio_core::ReadingTone::Warn,
        "兜底率 50% > 5% ⇒ 那一行需留意（面板不许把问题显示成没事）"
    );
    assert!(panel.has_warn(), "有 warn 行 ⇒ `has_warn()` 为真");

    // ④ 技能段：开关与已编译数都从那两列搬来（同源），不另算
    let skills = &panel.sections[3];
    let skill_row = skills.rows.first().expect("有技能行");
    assert_eq!(
        skill_row.value,
        if got.skill_compile_enabled {
            "开"
        } else {
            "关"
        },
        "开关行 = 列里的开关"
    );
    if got.skill_compile_enabled {
        assert!(
            skill_row
                .hint
                .as_deref()
                .unwrap_or("")
                .contains(&got.compiled_skills.to_string()),
            "已编译数要对上：{:?} vs {}",
            skill_row.hint,
            got.compiled_skills
        );
    }

    // ⑤ 承诺段：条数与违约数都从那列搬来（seed 没委托过 ⇒ 「没有委托」+ 平常语气）
    let cm_row = panel.sections[4].rows.first().expect("有承诺行");
    assert_eq!(
        got.commitments["total"].as_u64().unwrap_or(1),
        0,
        "seed 不产承诺"
    );
    assert_eq!(
        cm_row.value, "本会话没有委托",
        "零条要说人话，不显示「0 条 · 违约 0 条」"
    );
    assert_eq!(cm_row.tone, crate::symbio_core::ReadingTone::Normal);
}

/// 反向：**有违规 ⇒ 那一行变「需留意」**，否则面板把问题显示成没事。
///
/// 手术刀 = 复制一格开轮事件（N-C2 的「每轮至多一条开轮」立刻违规）。
#[test]
fn a_warn_row_appears_when_the_invariants_are_broken() {
    let wal = seed("panel-warn");
    {
        let raw = std::fs::read_to_string(&wal).expect("读 WAL");
        let first = raw
            .lines()
            .find(|l| l.contains("\"user.message\""))
            .expect("有开轮行")
            .to_string();
        // 同一 event_id 会被幂等拦下，故改 id 再追加（复制成「两条开轮」）
        let dup = first.replace("\"u0\"", "\"u0-copy\"");
        std::fs::write(&wal, format!("{raw}{dup}\n")).expect("追加重复开轮");
    }

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let inv = &got.panel.sections[2];
    let row = &inv.rows[0];
    assert_eq!(
        row.tone,
        crate::symbio_core::ReadingTone::Warn,
        "违规 ⇒ 需留意"
    );
    assert!(row.value != "0 条", "且不再显示 0 条：{}", row.value);
    assert!(got.panel.has_warn(), "`has_warn()` 必须为真");
}

/// 没有事实源 ⇒ 「在不在」那一行变成需留意（用户一眼看见「这个会话还没跑过」）。
#[test]
fn a_missing_wal_is_a_warn_row_not_a_silent_zero() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-nowal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let root = dir.clone();

    let got = read_path("s1", &wal, None, crate::symbio_core::PRINCIPAL_MAIN, &root)
        .expect("没有事实源不是错");
    assert!(!got.has_wal, "没有源");
    let row = got.panel.sections[1]
        .rows
        .iter()
        .find(|r| r.label == "事实源在不在")
        .expect("有在不在行");
    assert_eq!(row.value, "不在");
    assert_eq!(
        row.tone,
        crate::symbio_core::ReadingTone::Warn,
        "没跑过 ⇒ 需留意"
    );
}

/// 出口读数 == 直接 `apply`（复算：同源，不是「看起来差不多」）。
#[test]
fn stats_recompute_equals_direct_apply() {
    let wal = seed("recompute");
    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);

    assert!(got.has_wal);
    assert_eq!(got.session_id, "s1");

    // 读方复算：同一份切片、同一个 as-of，四列各自直接 apply。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let latency = slo_report()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let fallback = fallback_rate()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let cost = cost_ledger()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let ck = checkpoint()
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    let rs = serde_json::to_value(
        crate::symbio_core::readyset(crate::symbio_core::TASK_CAP)
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("就绪集序列化");
    assert_eq!(got.readyset, rs, "就绪集列同样复算（同切片、同 as-of）");

    assert_eq!(got.tiers.len(), 1, "种子数据只有一档");
    let row = &got.tiers[0];
    assert_eq!(row.tier, "deep");
    assert_eq!(row.turns, fallback.of("deep").turns);
    assert_eq!(row.fallbacks, fallback.of("deep").fallbacks);
    assert_eq!(row.rate, fallback.of("deep").rate());
    assert_eq!(row.p50, latency.of("deep").p50());
    assert_eq!(row.p95, latency.of("deep").p95());
    assert_eq!(row.p99, latency.of("deep").p99());
    assert_eq!(row.samples, latency.of("deep").count());
    assert_eq!(got.cost, serde_json::to_value(&cost).unwrap());
    assert_eq!(got.checkpoint, serde_json::to_value(&ck).unwrap());

    // 复算还得**算对**：期望值直接写在这里，不从出口自己身上取。
    assert_eq!((row.turns, row.fallbacks), (2, 1));
    assert_eq!(row.rate, 0.5);
    assert_eq!((row.samples, row.p95), (1, 1200), "兜底轮不进时延样本");
    assert_eq!(got.checkpoint["event_count"], 4);
    assert_eq!(got.cost["total_ms"], 4200, "两轮的 cost_ms 都入账");
}

/// 反向用例：删一格 ⇒ 对应那一列必须变（不是常数）。
#[test]
fn reverse_case_each_column_moves_when_the_wal_changes() {
    let wal = seed("reverse");
    assert_eq!(
        read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN).tiers[0].samples,
        1
    );

    // 删收束格 ⇒ 时延样本列归零、P95 归零
    drop_line(&wal, "f0");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(after.tiers[0].samples, 0, "删掉唯一的 final ⇒ 样本 1 → 0");
    assert_eq!(after.tiers[0].p95, 0, "无样本 ⇒ 分位数为 0（空档位不报警）");
    assert_eq!(after.tiers[0].turns, 2, "开轮格还在 ⇒ 分母不受收束格影响");
    assert_eq!(after.checkpoint["event_count"], 3, "断点是事实计数");

    // 删开轮格 ⇒ 兜底率的**分母**必须变（2 → 1），比率随之变成 1.0
    drop_line(&wal, "u0");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(
        after.tiers[0].turns, 1,
        "删掉一个 user.message ⇒ 分母 2 → 1"
    );
    assert_eq!(after.tiers[0].fallbacks, 1, "兜底格与被删的开轮格无关");
    assert_eq!(after.tiers[0].rate, 1.0, "分母变了 ⇒ 比率跟着变");
    assert_eq!(after.checkpoint["event_count"], 2);
}

/// 不变量列随事实源反向变化（[04 §3.1 批④](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// 真实形状的两轮全绿 ⇒ 空；删掉收束格 ⇒ C4 必须报出那个缺口。
#[test]
fn invariants_move_when_the_wal_changes() {
    let wal = seed("invariants");
    let clean = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);

    // 复算：出口的清单与直接 `check_all` 逐字相等（口径只活在 core，出口不复判）。
    // 切片口径 = 完整史（归档段 + 未取代那段，S2.7-b2）：出口读什么，复算就比什么。
    let store = EventWalStore::open_readonly(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    assert_eq!(
        clean.invariants,
        serde_json::to_value(check_all(&snap)).unwrap()
    );
    assert_eq!(
        clean.invariants,
        serde_json::json!([]),
        "两轮都收束、档位已声明且预算内 ⇒ 不变量清单为空（首日不假红）"
    );

    // 删轮 0 的收束格 ⇒ 轮 0 被轮 1 越过 ⇒ C4 报未收束；行删了 ⇒ C1 报 seq 跳号。
    drop_line(&wal, "f0");
    let list = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN).invariants;
    let list = list.as_array().expect("清单是数组");
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(
        list.iter()
            .any(|v| v["why"].as_str().unwrap_or_default().contains("未收束")),
        "C4：删掉收束格 ⇒ 该轮被判未收束：{list:?}"
    );
    assert!(
        list.iter().any(|v| v["event_id"] == "u0"),
        "违规锚在开轮那条（缺口本身，不是切片末尾某条无关事件）：{list:?}"
    );
    assert!(
        list.iter()
            .any(|v| v["why"].as_str().unwrap_or_default().contains("seq")),
        "C1：删掉中间一行 ⇒ 后续 seq 跳号：{list:?}"
    );
}

/// **归档不缩小读数**（S2.7-b2）：压缩把被取代的前缀移进归档段后，读数口拿到的
/// 仍是完整史（归档段 + 未取代那段），而不是那一小段窗口。
///
/// ## 只给窗口会错在两处（都是静默的）
///
/// 1. **C1 假红**：未取代那段的**首号不是 0**（它是全局号空间的一个窗口），
///    `seq_monotonic` 会把「应为 0 却得 4」报成违规 ⇒ 每次压缩后读数口凭空多一条；
/// 2. **四列缩水**：被归档的轮次仍然发生过（钱花过、兜底过），看不见它们会把
///    它们从分母 / 分子 / 样本里抹掉。
///
/// 按布局分叉的第二个读法（只给窗口）因此不是「多一个选择」，而是把布局知识
/// 交给每个调用方去挑——挑错还是静默的（[plan/14 §2.2](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 推论 1）。
///
/// ## 反向自检
///
/// 把 `stats::read` 里的 `store.range(Seq::new(0))` 换成「从最新快照起自切窗口」
/// （`super::store::current_window(...)`）⇒ 本用例红（`invariants` 非空、`tiers` 从 2 轮掉到 0 轮）。
#[test]
fn archival_does_not_shrink_the_readout() {
    let wal = seed("archive");
    let before = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(before.tiers.len(), 1, "两轮同档 ⇒ 一行");
    assert_eq!(before.tiers[0].turns, 2, "分母 = 两轮");
    assert_eq!(before.tiers[0].fallbacks, 1, "分子 = 一轮兜底");

    // 压缩：落一条快照，再把被取代的前缀整段归档（未取代那段只剩快照一条）。
    let store = EventWalStore::open(&wal).unwrap();
    let seq = store
        .append(
            Event::pending(
                "v2sc-3",
                EVENT_SESSION_CONSOLIDATED,
                Entity::Message,
                Verb::Progressed,
                0,
                "session/main",
            )
            .with_payload(serde_json::json!({
                "messages": [{ "id": "snap-1" }],
                "turns": 2,
            })),
        )
        .unwrap();
    assert_eq!(
        store.archive_before(seq).unwrap(),
        Some(4),
        "前缀 4 条全进归档段"
    );
    assert_eq!(store.len(), 1, "没被取代的那段只剩快照");
    drop(store);

    // 读数口仍看完整史：不变量不假红，四列与成本一分不缩。
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(
        after.invariants,
        serde_json::json!([]),
        "归档是号空间的窗口边界，不是「事件被丢弃」——C1 不该报违规"
    );
    assert_eq!(
        after.tiers, before.tiers,
        "分母 / 分子 / 样本不因归档而缩水"
    );
    assert_eq!(after.cost, before.cost, "成本台账看的是完整史");
}

/// 没有事实源 = 有据的零，且读方不许创建文件（真·只读）。
#[test]
fn missing_wal_is_an_honest_zero_and_creates_nothing() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-missing", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert!(!got.has_wal);
    assert!(got.tiers.is_empty());
    assert_eq!(got.checkpoint["event_count"], 0);
    assert_eq!(got.cost["total_ms"], 0);
    assert_eq!(got.invariants, serde_json::json!([]), "空事实源 ⇒ 断言全绿");
    assert_eq!(
        got.transcript,
        serde_json::json!({ "entries": [] }),
        "没有事实源 ⇒ 转写为空表（有据的空，不是缺列）"
    );
    assert!(
        !wal.exists(),
        "读方不得为了读而创建事实源文件——那会把「没跑过一轮」变成「跑过一轮空的」"
    );
}

/// 读侧闸（[04 §3.1 批⑥](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：声明**属主**
/// 身份 ⇒ 与不声明逐字相同——闸的存在不是为了把属主挡在外面。
#[test]
fn declaring_the_owner_reads_everything() {
    let wal = seed("gate-owner");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let plain = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let as_owner = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_USER),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("属主读数");
    assert_eq!(
        as_owner, plain,
        "属主本人的读数与本机默认逐字一致（声明不改变她能看什么）"
    );
    assert_eq!(as_owner.checkpoint["event_count"], 4, "4 格全见");
}

/// 读侧闸的反向：**非属主 / 矩阵外主体读数为空**（fail-closed）。
///
/// `has_wal` 仍为真 ⇒ 「有源但不给你看」与 `missing_wal` 那条「没有源」可分辨；
/// 不变量列同为空——没看见事实，就没有属于你的缺口可报。
///
/// 前提（② 之后要说清的一句）：本事实源**没有共享内容**——`seed` 只种
/// 对话格不种记忆，内容侧可见域因此是 C10 缺省 `thread_private`，闸的答案
/// 与接线前逐字相同。共享内容的读方见下一条。
#[test]
fn a_non_owner_reads_nothing_at_all() {
    let wal = seed("gate-nonowner");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    for viewer in ["session/main", "session/ghost"] {
        let got = read_path(
            "s1",
            &wal,
            Some(viewer),
            crate::symbio_core::PRINCIPAL_MAIN,
            &root,
        )
        .expect("读数");
        assert!(got.has_wal, "{viewer}: 事实源存在，只是不给你看");
        assert!(
            got.tiers.is_empty(),
            "{viewer}: 四列应为空：{:?}",
            got.tiers
        );
        assert_eq!(
            got.checkpoint["event_count"], 0,
            "{viewer}: 断点列的事件数为 0"
        );
        assert_eq!(got.cost["total_ms"], 0, "{viewer}: 成本列为 0");
        assert_eq!(
            got.invariants,
            serde_json::json!([]),
            "{viewer}: 没有可见事实 ⇒ 无可报的违规"
        );
        // 声誉列随四列一起没：**全有全无**，不按字段裁——按字段裁会留下
        // 「四列不给你、声誉给你」这种没有任何理由的半开半掩。
        assert_eq!(
            got.reputation,
            serde_json::json!({}),
            "{viewer}: 没有可见事实 ⇒ 不替你看不见的主体报数"
        );
        // 转写列同四列一起没（空切片 ⇒ `{entries: []}`）：别人的对话不该从你的读数里漏出去。
        assert_eq!(
            got.transcript,
            serde_json::json!({ "entries": [] }),
            "{viewer}: 没有可见事实 ⇒ 转写为空表"
        );
    }
}

/// ② 的判据：读侧闸传的是**内容侧**的可见域——本会话记忆载荷里记了
/// `shared`（人的认知可供借用，SC-010 判据 3）⇒ 矩阵内主体（`agent:main`）
/// 读得到他人共享内容；矩阵外主体（`agent:ghost`）照旧空切片。
///
/// 三条并排证的是同一件事的三个面：闸读的是**产地的事实**（载荷 `vis_scope`），
/// 不是恒传的缺省值、也不是调用方声明——请求方自己声明「我能看到什么」
/// 等于给自己授权，那条捷径 `governance` 模块头点名禁止。
///
/// 反向对照（同一次播种里自带）：把那条唯一共享记忆换成私有的 ⇒ `agent:main`
/// 回到空切片——证明翻面的是**内容的可见域**这一个变量，不是别的。
#[test]
fn a_matrix_member_reads_shared_content_and_an_outsider_still_cannot() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-gate-shared", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();
    let t = 1_700_000_000_000i64;
    // 一轮对话（与 `seed` 同形的最小事实源）+ 一条 **shared** 的经验记忆
    // （`derived_facts::record_learning` 步 11 的生产形态）。
    let u0 = Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好", "tier": "deep", "turn_ref": "u-0" }));
    let s0 = store.append(u0).unwrap().value();
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "session/main",
            )
            .with_produced_by(s0)
            .with_cost_ms(1200)
            .with_ts(t)
            .with_payload(serde_json::json!({ "text": "你好！", "tier": "deep" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "v2m-shared",
                crate::symbio_core::EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                "user",
            )
            .with_produced_by(s0)
            .with_ts(t)
            .with_payload(serde_json::json!({
                "content": "我喜欢喝不加糖的咖啡",
                "tag": "经验",
                "generation": 0u32,
                "vis_scope": "shared",
            })),
        )
        .unwrap();

    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let session_principal = crate::symbio_core::PRINCIPAL_MAIN;

    // ① 矩阵内主体（非属主）：读得到——共享内容让这份事实源不再纯私有。
    let as_member = read_path("s1", &wal, Some("session/main"), session_principal, &root)
        .expect("矩阵内主体读数");
    assert!(as_member.has_wal);
    assert_eq!(
        as_member.checkpoint["event_count"], 3,
        "矩阵内主体读得到他人共享内容（三格全见）"
    );
    assert_eq!(as_member.cost["total_ms"], 1200, "成本列同见");

    // ② 矩阵外主体：照旧空切片（fail-closed 不因共享内容松动）。
    let as_outsider = read_path("s1", &wal, Some("session/ghost"), session_principal, &root)
        .expect("矩阵外主体读数");
    assert!(as_outsider.has_wal, "有源但不给你看");
    assert_eq!(
        as_outsider.checkpoint["event_count"], 0,
        "矩阵外主体断点列为空"
    );
    assert_eq!(as_outsider.cost["total_ms"], 0);

    // ③ 属主本人：与不声明时逐字一致（闸不是为了把属主挡在外面）。
    let plain = read_path("s1", &wal, None, session_principal, &root).expect("本机默认读数");
    let as_owner = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_USER),
        session_principal,
        &root,
    )
    .expect("属主读数");
    assert_eq!(as_owner, plain);

    // 反向对照：唯一那条共享记忆改成私有 ⇒ agent:main 回到空切片。
    // （同一次播种、同一份代码，只翻内容的可见域这一个变量。）
    let mut events = store.range(Seq::new(0));
    let last = events.len() - 1;
    events[last].payload["vis_scope"] = serde_json::json!("thread_private");
    drop(store);
    std::fs::remove_file(&wal).unwrap();
    let store2 = EventWalStore::open(&wal).unwrap();
    for e in events {
        store2.append(e).unwrap();
    }
    let denied =
        read_path("s1", &wal, Some("session/main"), session_principal, &root).expect("读数");
    assert!(
        denied.tiers.is_empty() && denied.checkpoint["event_count"] == 0,
        "内容全私有 ⇒ 矩阵内非属主回到空切片（翻面的是可见域，不是别的）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 声誉列（[plan/12 批 2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")，S6 第 15 步的读侧）：
/// `own` 取**本会话主体**，`by_principal` 把这份事实源里的主体原样摊开（`own` 在其中，
/// 不另造口径）；判据只有一处——core 的 `reputation` 投影，本处只取数 + 排版。
#[test]
fn reputation_column_reports_own_and_by_principal() {
    let wal = seed("rep");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let user_seq = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格")
        .seq()
        .map(|s| s.value())
        .expect("已入格");
    // 一次守约的代际立约（写方 `derived_facts::record_derived` 落的就是这三格）。
    for e in crate::symbio_core::commitment_events(
        "c-rep",
        crate::symbio_core::PRINCIPAL_MAIN,
        crate::symbio_core::PRINCIPAL_USER,
        "把这段复查看完",
        true,
        "",
        user_seq,
    ) {
        store.append(e).expect("承诺入格");
    }

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(
        got.reputation["own"],
        serde_json::json!({
            "principal": crate::symbio_core::PRINCIPAL_MAIN,
            "offered": 1,
            "kept": 1,
            "broken": 0,
            "score": 1,
        }),
        "own = 本会话主体的条目；score = 平凡打分（守约 − 违约）"
    );
    assert_eq!(
        got.reputation["by_principal"][crate::symbio_core::PRINCIPAL_MAIN],
        got.reputation["own"],
        "own 就在 by_principal 里——同一张表的两个视角，不是两套数"
    );
    assert!(
        got.invariants.as_array().is_some_and(|l| l.is_empty()),
        "承诺四格不新增违规：{:?}",
        got.invariants
    );

    // 无读权限 ⇒ **整列为空对象**，不是零值条目（零值 = 替一个你看不见的主体报数）。
    let denied = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.reputation, serde_json::json!({}));
}

/// 承诺列（段二，S08 §5 的读侧出口）：
/// 一轮里委托过一次 ⇒ 这里就**读得出那一条**（谁向谁、承诺了什么、违约为什么）。
///
/// ## 这条判据补的是哪一半
///
/// `agent_run` 在能力元数据里自声明「我记委托账」，声明面的**名单封闭**由
/// `tool-decls-audit` 守着；但声明说得 ≠ 事实落了。写侧到格子的链条由
/// `turn_exec.test.rs::full_turn_lands_derived_commitment_facts` 钉（走真实通道，
/// 假 provider 回一个 `agent_run`），本用例钉读侧：格子 → 一条可查记录。
/// 两头之间只有 `commitment_events` 这一个成格口，夹具调的就是它——写侧改名，
/// 这两条一起红。
///
/// ## 三个方向
///
/// ① 违约必须带原因出现在读数里（"违约可被观测"是 S08 §5 的全部前提）；
/// ② 违约率的分母是**已了结**数：一条在途 + 一条违约 ⇒ 率 100%，不是 50%——
///   把没跑完的轮算进分母等于把「还在做」读成「没做到」；
/// ③ 无读权限 ⇒ 四列那一族的空形状（`{ items: [], total: 0 }`），不是缺列。
#[test]
fn commitments_column_exposes_each_delegation_record() {
    let wal = seed("commitments");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let store = EventWalStore::open(&wal).unwrap();
    let snap = store.range(Seq::new(0));
    let user_seq = snap
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格")
        .seq()
        .map(|s| s.value())
        .expect("已入格");
    // 一次违约的委托（写方 `record_derived` 落的正是这三格：立约 + 违约 + 告知）
    for e in crate::symbio_core::commitment_events(
        "v2c-t1-run-1",
        crate::symbio_core::PRINCIPAL_MAIN,
        "agent:reviewer",
        "让 reviewer 复查这段",
        false,
        "子智能体没起来",
        user_seq,
    ) {
        store.append(e).expect("承诺入格");
    }
    // 一条只立了约、还没了结的（在途）
    store
        .append(
            crate::symbio_core::commitment_events(
                "v2c-t2-run-1",
                crate::symbio_core::PRINCIPAL_MAIN,
                "agent:worker",
                "整理周报",
                true,
                "",
                user_seq,
            )[0]
            .clone(),
        )
        .expect("立约入格");

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let col = &got.commitments;
    assert_eq!(col["total"], 2, "两条约：一条违约、一条在途");
    assert_eq!(col["broken"], 1);
    assert_eq!(col["kept"], 0);
    assert_eq!(col["settled"], 1, "分母只算已了结的（在途那条不算）");
    assert!(
        col["breach_rate"].as_f64().unwrap_or(0.0) > 0.99,
        "违约率 = 违约 / 已了结 = 1.0：{:?}",
        col["breach_rate"]
    );
    let items = col["items"].as_array().expect("items 是数组");
    assert_eq!(
        items
            .iter()
            .map(|i| (
                i["id"].as_str().unwrap_or(""),
                i["state"].as_str().unwrap_or("")
            ))
            .collect::<Vec<_>>(),
        vec![("v2c-t1-run-1", "broken"), ("v2c-t2-run-1", "open"),],
        "按承诺号排序，同一份事实源永远读出同一个清单"
    );
    assert_eq!(items[0]["promise"], "让 reviewer 复查这段");
    assert_eq!(items[0]["to"], "agent:reviewer");
    assert_eq!(
        items[0]["why"], "子智能体没起来",
        "违约原因就是读数的一部分——用户在这里读「为什么没做到」"
    );
    assert_eq!(items[0]["anchor_seq"].as_u64(), Some(user_seq));
    assert!(
        check_all(&store.range(Seq::new(0))).is_empty(),
        "承诺格子不新增违规：{:?}",
        check_all(&store.range(Seq::new(0)))
    );

    // 面板那一份（人读的）与列同源：数字直接来自本列，不重算
    let row = &got.panel.sections[4].rows[0];
    assert!(
        row.value.contains("2 条 · 违约 1 条"),
        "面板与列对上：{:?}",
        row.value
    );
    assert_eq!(
        row.tone,
        crate::symbio_core::ReadingTone::Warn,
        "有违约 ⇒ 需留意"
    );

    // 无读权限 ⇒ 空形状（四列那一族），不是缺列、也不是零值条目
    let denied = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        denied.commitments["items"].as_array().map(|a| a.len()),
        Some(0),
        "无权读 ⇒ 切片为空 ⇒ 一条也没有（与「没委托过」同形，由 has_wal 分辨）"
    );
    assert_eq!(denied.commitments["total"], 0);
}

/// 就绪集列（S7 步 16，[04 §3.1 批⑨]）：候选集入列 + **复算同源** + 读侧闸（反向）。
///
/// `may_read = false ⇒ {ready: []}` 与四列同款（空切片 = 有源但不给你看），
/// 不是声誉那列的空对象——「没有任务」与「不给你看」由 `has_wal` 与列的上下游分辨。
#[test]
fn readyset_column_lists_candidates_and_obeys_the_read_gate() {
    let dir = std::env::temp_dir().join(format!("symbio-stats-rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let root = dir.clone();
    let store = EventWalStore::open(&wal).expect("WAL");
    for (id, deps) in [("t1", vec![]), ("t2", vec!["t1"])] {
        store
            .append(
                Event::pending(
                    id.to_string(),
                    EVENT_TASK_OPENED,
                    Entity::Task,
                    Verb::Opened,
                    0,
                    crate::symbio_core::PRINCIPAL_MAIN,
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({
                    "task_id": id,
                    "depends_on": deps,
                    "goal": id,
                })),
            )
            .expect("任务开格");
    }

    let got = read_path(
        "s-rs",
        &wal,
        None,
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        got.readyset["ready"].as_array().map(|a| a.len()),
        Some(1),
        "t1 无依赖（就绪）、t2 等 t1（未终态）：{}",
        got.readyset
    );
    assert_eq!(got.readyset["ready"][0]["task_id"], "t1");

    // 复算：同切片、同一个 as-of——出口只能调投影，不许另写一份统计。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        crate::symbio_core::readyset(crate::symbio_core::TASK_CAP)
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.readyset, direct, "就绪集列必须复算相等");

    // 反向：读侧闸关 ⇒ 空切片 ⇒ 连不变量列也空（有源但不给你看）。
    let denied = read_path(
        "s-rs",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(
        denied.readyset,
        serde_json::json!({ "ready": [], "cap": crate::symbio_core::TASK_CAP })
    );
    assert_eq!(denied.invariants, serde_json::json!([]));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 校准列（S9 步 22 的读侧，[plan/12 批 1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// 读数 == 直接 `apply`（复算）+ 随事实源**反向**变化（证明它在算，不是常数）。
///
/// 观测面 = 路由收束时落的 `memory.recalled{skill_id, fallback}`（`skill_events::route`
/// 的产物）——本处按**写方落下的真实形状**造（`Memory × Asserted`、`turn = 0`、
/// `produced_by` 指向本轮用户格），只把载荷换成校准认的那两把钥匙。
#[test]
fn calibration_column_recomputes_and_moves_with_the_wal() {
    let wal = seed("calibration");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    {
        let store = EventWalStore::open(&wal).expect("WAL");
        for (id, fallback) in [("calib-1", false), ("calib-2", true)] {
            store
                .append(
                    Event::pending(
                        id,
                        // 校准观测走**自己的格子**。旧写法把它写在
                        // `memory.recalled` 上、靠载荷字段认——那正是读侧赌「别的格子
                        // 恰好不带这两个字段」的那段。
                        EVENT_MEMORY_CALIBRATED,
                        Entity::Memory,
                        Verb::Asserted,
                        0,
                        "session/main",
                    )
                    .with_produced_by(0)
                    .with_payload(serde_json::json!({
                        "skill_id": "sk-a",
                        "fallback": fallback,
                    })),
                )
                .expect("路由观测入格");
        }
    }

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);

    // 复算：出口的校准列与直接 `apply` 逐字相等（口径只活在 core，出口不复判）。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        calibration()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.calibration, direct, "校准列必须复算相等");
    // 口径：sk-a 用了 2 次、其中回退 1 次（`confidence = 1 − 回退率` 由 core 算）。
    assert_eq!(got.calibration["by_skill"]["sk-a"]["uses"], 2);
    assert_eq!(got.calibration["by_skill"]["sk-a"]["fallbacks"], 1);

    // 反向：删掉回退那条 ⇒ 回退数 1 → 0（读数跟着事实源动）。
    drop_line(&wal, "calib-2");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(after.calibration["by_skill"]["sk-a"]["uses"], 1);
    assert_eq!(after.calibration["by_skill"]["sk-a"]["fallbacks"], 0);

    // 读侧闸：非属主 ⇒ 空切片 ⇒ `{ by_skill: {} }`（与就绪集列同族，不是声誉的空对象）。
    let denied = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.calibration, serde_json::json!({ "by_skill": {} }));
    assert!(denied.has_wal, "has_wal 独立于可见域");
}

/// 技能编译两列：`skill_compile_enabled`（部署开关）与 `compiled_skills`（本会话
/// `memory.encoded{tag:"skill"}` 的条数）。
///
/// 判据是**两种零必须分得开**：关着 ⇒ `false`（配置选择）；开着但没编译 ⇒ `true` +
/// `0`（运行结果）。计数不新增投影，只数事实源里已有的格子；读侧闸对它同样生效
/// （无权 ⇒ 空切片 ⇒ 0），开关是部署事实、不随闸变化。
#[test]
fn skill_compile_columns_distinguish_off_from_on_but_nothing_compiled() {
    let wal = seed("skills");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let read_with = |enabled: bool, viewer: Option<&str>| {
        read(
            "s1",
            facts_of(&wal),
            || fleet_of(&root),
            viewer,
            crate::symbio_core::PRINCIPAL_MAIN,
            enabled,
            crate::symbio_core::TASK_CAP,
        )
        .expect("读数")
    };

    // 关着：事实源里也没有技能格 ⇒ 两列各自如实
    let off = read_with(false, None);
    assert!(!off.skill_compile_enabled);
    assert_eq!(off.compiled_skills, 0);

    // 开着但本会话还没编译过 ⇒ 与上一行并读才分得清（两行计数相同、开关不同）
    let on_empty = read_with(true, None);
    assert!(on_empty.skill_compile_enabled);
    assert_eq!(on_empty.compiled_skills, 0);

    // 落两条技能 + 一条普通记忆：按 tag 认，不是按整格数
    {
        let store = EventWalStore::open(&wal).expect("WAL");
        for (id, tag) in [("sk-1", SKILL_TAG), ("sk-2", SKILL_TAG), ("mem-1", "fact")] {
            store
                .append(
                    Event::pending(
                        id,
                        EVENT_MEMORY_ENCODED,
                        Entity::Memory,
                        Verb::Opened,
                        0,
                        "user",
                    )
                    .with_produced_by(0)
                    .with_payload(serde_json::json!({ "tag": tag, "trigger": id, "content": id })),
                )
                .expect("记忆入格");
        }
    }

    let on = read_with(true, None);
    assert_eq!(on.compiled_skills, 2, "只有 tag=skill 的两条算技能");

    // 读侧闸：矩阵外主体 ⇒ 空切片 ⇒ 计数 0；开关是部署事实，照旧如实报告。
    let denied = read_with(true, Some(crate::symbio_core::PRINCIPAL_MAIN));
    assert_eq!(denied.compiled_skills, 0);
    assert!(denied.skill_compile_enabled);
    assert!(denied.has_wal, "has_wal 独立于可见域");
}

/// 转写列（**对话面读侧**，[plan/12 批 2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// 读的是**事实源**（`user.message` / `chat.assistant.final` / `chat.assistant.fallback` /
/// `artifact.added` 四格）而不是会话存储里的消息副本。
///
/// 判据 = 复算同源（口径只活在 core 的 `transcript` 投影）+ 期望值钉死 + 反向
/// （删一格 ⇒ 那一句从转写里消失）+ 读侧闸（非属主 ⇒ `{entries: []}`，四列形态）。
#[test]
fn transcript_column_reads_the_conversation_back_from_the_wal() {
    let wal = seed("transcript");
    let root = wal.parent().expect("WAL 有父目录").to_path_buf();
    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);

    // 复算：出口的转写列与直接 `apply` 逐字相等（口径只活在 core 的投影，出口不复判）。
    let ro = EventWalStore::open_readonly(&wal).expect("WAL 只读");
    let snap = ro.range(Seq::new(0));
    let direct = serde_json::to_value(
        transcript()
            .apply(&snap, i64::MAX, Budget::generous())
            .value,
    )
    .expect("序列化");
    assert_eq!(got.transcript, direct, "转写列必须复算相等");

    // 期望值直接写在这里（不从出口自己身上取）：种子的四格 → user/assistant 交替两条。
    // 兜底话术（`chat.assistant.fallback` 的 `why`）是用户**实际看到**的回复，也算 assistant 行。
    assert_eq!(
        got.transcript["entries"],
        serde_json::json!([
            { "role": "user", "text": "你好" },
            { "role": "assistant", "text": "答" },
            { "role": "user", "text": "再问" },
            { "role": "assistant", "text": "模型超时" },
        ]),
        "转写按事件顺序读出两条问答"
    );

    // 反向：删掉成功轮的收束格 ⇒ 对应那一句必须从转写里消失（证明它真的在读文件，不是常数）。
    drop_line(&wal, "f0");
    let after = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let entries = after.transcript["entries"].as_array().expect("条目数组");
    assert_eq!(entries.len(), 3, "删一条 final ⇒ 转写 4 → 3：{entries:?}");
    assert!(
        !entries.iter().any(|e| e["text"] == "答"),
        "被删的那句不得再出现在转写里：{entries:?}"
    );

    // 读侧闸：非属主 ⇒ 空切片 ⇒ `{ entries: [] }`（与就绪集 / 校准同族，不是声誉的空对象）。
    let denied = read_path(
        "s1",
        &wal,
        Some(crate::symbio_core::PRINCIPAL_MAIN),
        crate::symbio_core::PRINCIPAL_MAIN,
        &root,
    )
    .expect("读数");
    assert_eq!(denied.transcript, serde_json::json!({ "entries": [] }));
    assert!(denied.has_wal, "has_wal 独立于可见域：文件在，只是不给你看");
}

/// 转写列把**工具结果**也读出来（plan/10 批 3）：
/// `artifact.added` 进投影 ⇒ 列里多一条 `role = "tool"`（带 `tool` = 工具名）。
///
/// 这是「口径只活在 core 投影」的直接后果——出口不加任何判断，投影多一格，列就多一行。
/// 判据独立于上一条（上一条的种子不含工具轮）：这条**补一格工具产物**再看列，因此它
/// 证的是「列真的会浮出工具行」，不是「列恒等于种子那四行」。
#[test]
fn transcript_column_surfaces_tool_rows() {
    let wal = seed("transcript-tool");
    // 往事实源补一格工具产物（种子本身不含工具轮）。`produced_by` 指向轮 0 用户格（seq 0）。
    let store = EventWalStore::open(&wal).expect("WAL");
    store
        .append(
            Event::pending(
                "a0",
                EVENT_ARTIFACT_ADDED,
                Entity::Artifact,
                Verb::Asserted,
                0,
                "session/main",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "tool": "vdfs_read", "text": "文件内容" })),
        )
        .expect("追加工具格");

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let entries = got.transcript["entries"].as_array().expect("条目数组");
    let tool_row = entries
        .iter()
        .find(|e| e["role"] == "tool")
        .unwrap_or_else(|| panic!("转写列应含工具行：{entries:?}"));
    assert_eq!(tool_row["tool"], "vdfs_read", "工具行带工具名");
    assert_eq!(tool_row["text"], "文件内容", "工具行带结果正文");
    // 非工具行**不带** `tool`（`skip_serializing_if`：新增角色不改旧角色的线格式）。
    assert!(
        entries
            .iter()
            .filter(|e| e["role"] != "tool")
            .all(|e| e.get("tool").is_none()),
        "非工具行不该带 tool：{entries:?}"
    );
}

// ── 用量列与面板那一段（P2-4 的后端半边）──────────────────────────────

/// 一条 `response` 事实（端口 / 主题 / 可选实测用量）。
fn resp_event(id: &str, port: &str, principal: &str, usage: Option<(u64, Option<u64>)>) -> Event {
    let mut payload = serde_json::json!({
        "port": port,
        "id": id,
        "attempt": 1,
        "principal": principal,
    });
    if let Some((input, output)) = usage {
        let mut u = serde_json::json!({ "input": input });
        if let Some(o) = output {
            u["output"] = serde_json::json!(o);
        }
        payload["usage"] = u;
    }
    Event::pending(id, "response", Entity::Turn, Verb::Asserted, 0, "system")
        .with_produced_by(1)
        .with_payload(payload)
}

/// 一条**带金额**的 `response`（金额写入与用量同一次测量，见 `facts.rs`）。
fn spend_event(id: &str, principal: &str, currency: &str, amount_nano: u64) -> Event {
    let mut e = resp_event(id, "model", principal, Some((10, Some(5))));
    e.payload["spend"] = serde_json::json!({
        "amount_nano": amount_nano,
        "currency": currency,
    });
    e
}

fn wal_with(tag: &str, events: Vec<Event>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-stats-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal).unwrap();
    for e in events {
        store.append(e).unwrap();
    }
    wal
}

/// 判据：**「没测量」与「测到零」不是一件事**。
///
/// 三件事一起才叫判据：
///
/// 1. 只有带 `usage` 键的 `response{port=model}` 进账——不带用量的应答既不贡献
///    token，也不进 `calls`（`calls` 数的是「测到过几次」，不是「发了几次请求」）；
/// 2. 只报了一侧（有的协议只有 `output_tokens`）时另一侧读 0 而不是丢条目；
/// 3. 一次都没测到 ⇒ 面板上空**没有**「用量」这一段——写一行「输入 0」等于替
///    provider 编一个它没给的数（与上下文水位缺数整条不出现同一条纪律）。
#[test]
fn the_usage_column_counts_only_measured_responses() {
    let wal = wal_with(
        "token",
        vec![
            resp_event("m-r0", "model", "session/main", Some((1000, Some(200)))),
            resp_event("m-r1", "model", "sub/worker", Some((500, None))),
            // provider 没报 `usage`：不进账。
            resp_event("m-r2", "model", "session/main", None),
            // 别的端口的应答不带用量：混进来会把「一次往返」算成一次模型调用。
            resp_event("c-a", "tool", "session/main", None),
        ],
    );

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(got.tokens["calls"], serde_json::json!(2), "{}", got.tokens);
    assert_eq!(got.tokens["input"], serde_json::json!(1500));
    assert_eq!(got.tokens["output"], serde_json::json!(200));
    // 只报一侧的那一笔：另一侧是 0，条目仍在（按主体摊得开）。
    assert_eq!(
        got.tokens["by_principal"]["sub/worker"]["output"],
        serde_json::json!(0)
    );
    assert_eq!(
        got.tokens["by_principal"]["sub/worker"]["measured"],
        serde_json::json!(1)
    );

    let usage = got
        .panel
        .sections
        .iter()
        .find(|s| s.title.as_deref() == Some("用量"))
        .expect("测到过用量就必须有这一段");
    let total = usage
        .rows
        .iter()
        .find(|r| r.label == "合计")
        .expect("有合计行");
    assert_eq!(
        total.value, "1.7k",
        "1500 + 200 走 format_tokens 的唯一实现"
    );

    // 会话头那一格与这一段是**同一次装配**（两处调 `usage_section` 一个函数）：
    // 行必须逐行相等。差一行就是两份口径，而两份口径漂了没人会红——
    // 界面上只是「抽屉里四个数、头上一个数」，两边都自称是同一个会话的用量。
    assert_eq!(
        got.usage.as_ref().map(|s| &s.rows),
        Some(&usage.rows),
        "会话头的用量格必须与面板里那一段同源"
    );

    // 反向：一条带用量的都不给 ⇒ 整段不出现。
    let none = wal_with(
        "token-none",
        vec![resp_event("m-r0", "model", "session/main", None)],
    );
    let bare = read_at(&none, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(bare.tokens["calls"], serde_json::json!(0));
    assert!(
        !bare
            .panel
            .sections
            .iter()
            .any(|s| s.title.as_deref() == Some("用量")),
        "一次都没测到就不该有这一段：{:?}",
        bare.panel.sections
    );
    assert!(
        bare.usage.is_none(),
        "缺数时会话头那一格整格不出现，而不是画一条全零的读数"
    );
}

/// 判据：钱是**第三种量**——按币种分桶、桶内按主体摊开；缺席不冒充 0 元。
///
/// 1. 同一币种跨主体相加，不同币种各自一桶（币种顺序由 BTreeMap 给）；
/// 2. 面板一个币种一行（`花费 · USD`），金额由产者格式化到 1e-4；
/// 3. 会话头那一格与面板那一段仍是**同一次装配**（逐行相等）。
#[test]
fn the_spend_column_is_grouped_by_currency_and_principals() {
    let wal = wal_with(
        "spend",
        vec![
            spend_event("m-r0", "session/main", "usd", 1_500_000),
            spend_event("m-r1", "sub/worker", "  USD ", 500_000),
            spend_event("m-r2", "session/main", "CNY", 12_345_000),
            // 明确免费：币种在、金额 0 —— 进账（charges）但不加数。
            spend_event("m-r3", "session/main", "CNY", 0),
        ],
    );

    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    let usd = &got.spend["by_currency"]["USD"];
    assert_eq!(
        usd["amount_nano"],
        serde_json::json!(2_000_000),
        "{}",
        got.spend
    );
    assert_eq!(usd["charges"], serde_json::json!(2));
    assert_eq!(
        usd["by_principal"]["sub/worker"]["amount_nano"],
        serde_json::json!(500_000)
    );
    let cny = &got.spend["by_currency"]["CNY"];
    assert_eq!(cny["amount_nano"], serde_json::json!(12_345_000));
    assert_eq!(
        cny["charges"],
        serde_json::json!(2),
        "零金额也是一次带定价的往返"
    );

    let section = got
        .panel
        .sections
        .iter()
        .find(|s| s.title.as_deref() == Some("用量与花费"))
        .expect("有钱的行时标题要说出「花费」");
    let row = |label: &str| {
        section
            .rows
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("缺少 {label} 行：{:?}", section.rows))
    };
    assert_eq!(row("花费 · USD").value, "0.0020");
    assert_eq!(row("花费 · CNY").value, "0.0123");
    assert_eq!(
        got.usage.as_ref().map(|s| &s.rows),
        Some(&section.rows),
        "会话头的用量格必须与面板里那一段同源"
    );
}

/// 有用量、没定价 ⇒ 只有「用量」三行：不凭空写一行「0 元」，标题也不预告花费。
#[test]
fn usage_without_pricing_does_not_report_zero_money() {
    let wal = wal_with(
        "spend-none",
        vec![resp_event(
            "m-r0",
            "model",
            "session/main",
            Some((1000, Some(200))),
        )],
    );
    let got = read_at(&wal, crate::symbio_core::PRINCIPAL_MAIN);
    assert_eq!(got.spend["by_currency"], serde_json::json!({}));
    let section = got
        .panel
        .sections
        .iter()
        .find(|s| s.title.as_deref() == Some("用量"))
        .expect("没定价不影响 token 那三行");
    assert!(
        section.rows.iter().all(|r| !r.label.starts_with("花费")),
        "没定价就没有金额可报：{:?}",
        section.rows
    );
}
