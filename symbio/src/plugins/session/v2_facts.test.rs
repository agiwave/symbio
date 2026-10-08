//! v2 派生事实写入的验收——承诺 / 任务表 / 熔断 + 记忆与学习落进同一份事实源后，
//! 不变量全绿、溯源（N5）到位。
//!
//! ## 与「轮次事实」的关系
//!
//! 本模块**不写**轮次事实（`u-{turn}` 开口 / 收束格）——那两格由 core 的
//! `TurnRunner` 在 v2 运行器里原生入格。本模块只接在其后写**派生副作用**。
//! 所以夹具 [`seed_turn`] 自己播那两格：它要的就是「开口与收束已在事实源里」这个
//! 前置，播法与 `TurnRunner` 同形（事件号 `u-{turn}` / `f-{turn}`、溯源指本轮开口）。
//!
//! ## 这里不再覆盖什么
//!
//! - **轮次事实本身**（开口 / 收束 / 成本 / 重试 attempt）：那是运行器的面，判据在
//!   `v2_exec.test.rs`；
//! - **记忆三段的端到端接线**：`v2_exec.test.rs::full_turn_lands_memory_and_learning_facts`
//!   带**反向自检**（注掉运行器里的 `record_learning` 调用本用例必须红）。本文件
//!   只钉 `record_learning` 自己的口径，尤其是**巩固要用刷新后的快照**（编码刚写下
//!   的那条必须对巩固可见）——那一层端到端看不见。

use super::{record_derived, record_learning};
use crate::symbio_core::{
    check_all, recall, Budget, Entity, Event, EventEnvelope as _, EventWalStore, Seq, Store, Verb,
    EVENT_ASSISTANT_FINAL, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
    EVENT_MEMORY_RECALLED, EVENT_USER_MESSAGE,
};
use std::path::{Path, PathBuf};

/// 一个测试的临时目录：**用前先删**，上次残留不污染本次（Windows 会复用 pid）。
///
/// 收尾的 `remove_dir_all` 只在**用例跑通**时执行；用例一旦 panic，残留的
/// `v2-events.wal` 就会跟着下次 pid 复用回来，把确定事件 id 撞成 `Duplicate`。
fn tmp_wal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-v2facts-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    dir.join("v2-events.wal")
}

/// 播下「本轮开口 + 收束」两格，返回 `(store, snapshot, user_seq)`。
///
/// 形状照 core `TurnRunner`（`u-{turn}` / `f-{turn}`、收束格溯源指本轮开口），
/// **不是**照旧的转写形状（那是退役前的 `v2u-*-a{n}` 一族）。
fn seed_turn(
    wal: &Path,
    principal: &str,
    turn: u64,
    text: &str,
    cost_ms: u64,
) -> (EventWalStore, Vec<Event>, u64) {
    let store = EventWalStore::open(wal).expect("打开 WAL");
    let user_seq = store
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                crate::symbio_core::authz::PRINCIPAL_USER,
            )
            .with_payload(serde_json::json!({ "text": text, "tier": "deep" })),
        )
        .expect("开口入格")
        .value();
    store
        .append(
            Event::pending(
                format!("f-{turn}"),
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                turn,
                principal,
            )
            .with_produced_by(user_seq)
            .with_cost_ms(cost_ms)
            .with_payload(serde_json::json!({ "text": "答", "model": "deep" })),
        )
        .expect("收束入格");
    let snapshot = store.range(Seq::new(0));
    (store, snapshot, user_seq)
}

/// 重开恢复（N2）：关掉再开，事件还在、seq 完好、不变量照绿。
#[test]
fn reopen_recovers_events_with_seq() {
    let wal = tmp_wal("reopen");
    let (store, snap, _) = seed_turn(&wal, crate::symbio_core::authz::PRINCIPAL_MAIN, 0, "问", 5);
    let count_before = snap.len();
    drop(store);

    let reopened = EventWalStore::open(&wal).unwrap();
    let snap = reopened.range(Seq::new(0));
    assert_eq!(snap.len(), count_before, "重开后事件不丢");
    assert!(
        snap.iter().all(|e| e.seq().is_some()),
        "重开恢复的事件必须带 seq"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 代际立约（[04 §3.1 批⑧](../../../../docs/plan/04-工程落地.md)，S08 §3「加格子，不加机制」）：
/// 本轮的 `agent_run` 委托落成 `commitment.opened` → `released`，**同锚在本轮开口**。
#[test]
fn settled_delegation_becomes_commitment_opened_then_released() {
    use super::super::tools::Delegation;

    let wal = tmp_wal("commit-ok");
    let (store, snapshot, user_seq) = seed_turn(
        &wal,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        0,
        "交给子智能体",
        50,
    );
    let delegations = [Delegation {
        id: "call-1".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: true,
        why: String::new(),
    }];
    record_derived(
        &wal,
        &store,
        &snapshot,
        0,
        user_seq,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "t0",
        &delegations,
        &[],
        &[],
    )
    .expect("派生事实入格");

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let opened = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_OFFERED)
        .expect("立约格");
    let released = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_RELEASED)
        .expect("守约格");

    assert_eq!(opened.entity, Entity::Commitment);
    assert_eq!(opened.produced_by, Some(user_seq), "立约锚在本轮开口上");
    assert_eq!(
        released.produced_by,
        Some(user_seq),
        "立约与了结同锚成对——`check_all` 才看得见它们是一对"
    );
    assert_eq!(
        released.payload.get("id"),
        opened.payload.get("id"),
        "了结按载荷 id 找回立约方（收束时不重抄 `from`）"
    );
    assert_eq!(
        opened.payload.get("from").and_then(|v| v.as_str()),
        Some(crate::symbio_core::authz::PRINCIPAL_MAIN),
        "承诺方 = 会话主体（声誉记在承诺方头上）"
    );
    assert_eq!(
        opened.payload.get("to").and_then(|v| v.as_str()),
        Some(crate::symbio_core::authz::PRINCIPAL_USER),
        "承诺对象 = 会话外的另一方"
    );
    assert_eq!(
        opened.payload.get("promise").and_then(|v| v.as_str()),
        Some("让 reviewer 复查这段"),
        "承诺内容 = 委托出去的那句话"
    );
    // 承诺号带 `anchor_id` 前缀：调用编号只在**一次模型响应内**唯一，而 WAL 的幂等键
    // 是事件 id——不加前缀，跨轮复用同一编号会让第二次立约撞 `Duplicate`、把整轮拖失败。
    assert!(
        opened.event_id.starts_with("c-offer-v2c-t0-"),
        "承诺号须带轮次前缀：{}",
        opened.event_id
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// 违约必须可观测（S08 §5）：`broken` 带 `why`，并**宣告**给承诺对象——
/// 宣告仍是同一份事实源里的一格，不是新通道（S08 §2「通信 = 没有直连」）。
#[test]
fn breached_delegation_is_declared_to_the_counterparty() {
    use super::super::tools::Delegation;

    let wal = tmp_wal("commit-breach");
    let (store, snapshot, user_seq) = seed_turn(
        &wal,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        0,
        "交给子智能体",
        50,
    );
    let delegations = [Delegation {
        id: "call-2".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: false,
        why: "子会话中途失败".to_string(),
    }];
    record_derived(
        &wal,
        &store,
        &snapshot,
        0,
        user_seq,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "t0",
        &delegations,
        &[],
        &[],
    )
    .expect("派生事实入格");

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let broken = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_BROKEN)
        .expect("违约收束必须成格");
    assert_eq!(
        broken.payload.get("why").and_then(|v| v.as_str()),
        Some("子会话中途失败"),
        "违约必须带 why（可观测，S08 §5）"
    );

    let asserted = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_COMMITMENT_ASSERTED)
        .expect("违约宣告必须成格");
    assert_eq!(asserted.entity, Entity::Commitment);
    let statement = asserted
        .payload
        .get("statement")
        .and_then(|v| v.as_str())
        .expect("宣告载荷带原话");
    assert!(
        statement.contains(crate::symbio_core::authz::PRINCIPAL_USER)
            && statement.contains("子会话中途失败"),
        "宣告 = 把这次违约告知承诺对象：{statement}"
    );
    assert_eq!(
        asserted.produced_by,
        Some(user_seq),
        "宣告同样锚在本轮开口上"
    );

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}

/// `Break` 的理由落成 `control × opened`，溯源锚是本轮开口；`breaks` 为空 ⇒ 一格都不落。
///
/// 这是 [roadmap/S09 §6](../../../../docs/plan/roadmap/S09-外部执行与熔断.md) 的两条
/// **相反**验收：验收 1（未授权 ⇒ 拒绝且不产生事件）与验收 2（预算耗尽 ⇒ 必须产出
/// 熔断事件）。两条必须由**同一个出参**分别触发——有理由就落、没理由就不落；
/// 「反正都落一条」违反 1，「反正都不落」违反 2。
#[test]
fn break_reason_lands_as_a_control_event_and_empty_stays_silent() {
    // ── 有理由：落格 ────────────────────────────────────────────────────
    let wal = tmp_wal("break");
    let (store, snapshot, user_seq) = seed_turn(
        &wal,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        0,
        "让它跑一遍",
        7,
    );
    record_derived(
        &wal,
        &store,
        &snapshot,
        0,
        user_seq,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "t0",
        &[],
        &[],
        &["budget-exhausted"],
    )
    .expect("派生事实入格");

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    let brk = snap
        .iter()
        .find(|e| e.entity == Entity::Control && e.verb == Verb::Opened)
        .expect("熔断必须成格（S09 §6 验收 2：不允许静默继续）");
    assert_eq!(
        brk.payload.get("reason").and_then(|v| v.as_str()),
        Some("budget-exhausted"),
        "载荷的 reason 区分打断与熔断"
    );
    assert_eq!(
        brk.produced_by,
        Some(user_seq),
        "溯源锚 = 本轮开口（与承诺 / 任务同锚）"
    );
    std::fs::remove_dir_all(wal.parent().unwrap()).ok();

    // ── 没理由（`Refuse` 或未触发）：一格都不落 ────────────────────────
    let quiet = tmp_wal("break-none");
    let (store, snapshot, user_seq) = seed_turn(
        &quiet,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        0,
        "问",
        1,
    );
    record_derived(
        &quiet,
        &store,
        &snapshot,
        0,
        user_seq,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "t0",
        &[],
        &[],
        &[],
    )
    .expect("派生事实入格");
    let snap = EventWalStore::open(&quiet)
        .expect("重开")
        .range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    assert!(
        !snap.iter().any(|e| e.entity == Entity::Control),
        "没有熔断理由 ⇒ 不产生任何事件（S09 §6 验收 1）"
    );
    std::fs::remove_dir_all(quiet.parent().unwrap()).ok();
}

/// 记忆三段（步 11 编码 / 步 12 检索锚 / 步 13 巩固）全部在同一份 WAL 里可读，
/// 且 `check_all` 全绿（溯源 100%）。
///
/// 端到端接线由 `v2_exec.test.rs::full_turn_lands_memory_and_learning_facts` 带反向
/// 自检覆盖；本用例钉的是**巩固要用刷新后的快照**——步 11 刚写下的那条必须对步 13
/// 可见（先巩固后编码会让第 4 条永远等不到代数 1）。
#[test]
fn record_learning_wires_all_three_memory_steps() {
    let wal = tmp_wal("memory-wiring");

    // 轮 0–3：每轮说一句、四句互不相同 ⇒ 第 4 轮收束时活记忆到 4 条 ⇒ 巩固触发。
    for i in 0..4u64 {
        let (store, snapshot, user_seq) = seed_turn(
            &wal,
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            i,
            &format!("第 {i} 条约定"),
            i,
        );
        record_learning(
            &wal,
            &store,
            &snapshot,
            i,
            user_seq,
            &format!("第 {i} 条约定"),
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            &format!("t{i}"),
            Some("答"),
            None,
            &[],
            false,
            crate::symbio_core::clock_now_ms(),
        );
    }

    let store = EventWalStore::open(&wal).expect("打开 WAL");
    let snap = store.range(Seq::new(0));
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED)
            .count(),
        4,
        "步 11：每轮各编码一条"
    );
    snap.iter()
        .find(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .expect("步 13：第 4 条到齐即触发巩固（编码先于巩固 ⇒ 刚写的那条对巩固可见）")
        .payload
        .get("generation")
        .and_then(|v| v.as_u64())
        .filter(|g| *g == 1)
        .expect("首次合并代数 = 1");
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_FORGOTTEN)
            .count(),
        2,
        "最旧的两条源记忆被排除式遗忘（Log 不删，只是投影不再包含）"
    );

    // 步 12：视图在收束前读出，收束时才落成事实（溯源锚那时才存在）。
    let view = recall(crate::symbio_core::authz::PRINCIPAL_USER, None)
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    assert!(!view.entries.is_empty(), "还有活记忆可召回");
    drop(store);

    let (store, snapshot, user_seq) = seed_turn(
        &wal,
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        4,
        "第五句",
        44,
    );
    record_learning(
        &wal,
        &store,
        &snapshot,
        4,
        user_seq,
        "第五句",
        crate::symbio_core::authz::PRINCIPAL_MAIN,
        "t4",
        Some("答"),
        Some(&view),
        &[],
        false,
        crate::symbio_core::clock_now_ms(),
    );

    let snap = EventWalStore::open(&wal).expect("重开").range(Seq::new(0));
    let recalled = snap
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_RECALLED)
        .expect("步 12：检索事实入格");
    let fifth_user = snap
        .iter()
        .rfind(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("本轮用户格");
    assert_eq!(
        recalled.produced_by,
        fifth_user.seq().map(|s| s.value()),
        "溯源锚 = 触发检索的那格用户发言"
    );
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    std::fs::remove_dir_all(wal.parent().unwrap()).ok();
}
