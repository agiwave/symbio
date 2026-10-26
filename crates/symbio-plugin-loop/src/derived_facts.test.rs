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
//!   `turn_exec.test.rs`；
//! - **记忆三段的端到端接线**：`turn_exec.test.rs::full_turn_lands_memory_and_learning_facts`
//!   带**反向自检**（注掉运行器里的 `record_learning` 调用本用例必须红）。本文件
//!   只钉 `record_learning` 自己的口径，尤其是**巩固要用刷新后的快照**（编码刚写下
//!   的那条必须对巩固可见）——那一层端到端看不见。

use super::{record_derived, record_elimination, record_learning};
use symbio_core::{
    check_all, recall, Budget, Entity, Event, EventEnvelope as _, EventStore, Seq, Store, Verb,
    EVENT_ASSISTANT_FINAL, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
    EVENT_MEMORY_RECALLED, EVENT_USER_MESSAGE, EVENT_VERDICT_ASSERTED,
};

/// 播下「本轮开口 + 收束」两格，返回 `(snapshot, user_seq)`。
///
/// 形状照 core `TurnRunner`（`u-{turn}` / `f-{turn}`、收束格溯源指本轮开口），
/// **不是**照旧的转写形状（那是退役前的 `v2u-*-a{n}` 一族）。
fn seed_turn(
    store: &EventStore,
    principal: &str,
    turn: u64,
    text: &str,
    cost_ms: u64,
) -> (Vec<Event>, u64) {
    let user_seq = store
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                symbio_core::PRINCIPAL_USER,
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
    (snapshot, user_seq)
}

/// 代际立约（[04 §3.1 批⑧](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")，S08 §3「加格子，不加机制」）：
/// 本轮的 `agent_run` 委托落成 `commitment.opened` → `released`，**同锚在本轮开口**。
#[test]
fn settled_delegation_becomes_commitment_opened_then_released() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
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
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &delegations,
        &[],
        &[],
    );

    let snap = store.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let opened = snap
        .iter()
        .find(|e| e.kind == symbio_core::EVENT_COMMITMENT_OFFERED)
        .expect("立约格");
    let released = snap
        .iter()
        .find(|e| e.kind == symbio_core::EVENT_COMMITMENT_RELEASED)
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
        Some(symbio_core::PRINCIPAL_MAIN),
        "承诺方 = 会话主体（声誉记在承诺方头上）"
    );
    assert_eq!(
        opened.payload.get("to").and_then(|v| v.as_str()),
        Some(symbio_core::PRINCIPAL_USER),
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
}

/// `to` 是**数据**（[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")， 的单测判据）：
/// 来源主体由调用方传入——对等主体向「把消息交进来的那个主体」立约，不再写死
/// 本机用户。缺省回落 `PRINCIPAL_USER` 的行为由上面的用例钉住（传入的就是它）。
#[test]
fn the_commitment_counterparty_is_data_not_a_constant() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_PEER,
        0,
        "对等主体跑一轮",
        50,
    );
    let delegations = [Delegation {
        id: "call-peer-1".to_string(),
        promise: "把结果交给主智能体".to_string(),
        ok: true,
        why: String::new(),
    }];
    record_derived(
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_PEER,
        symbio_core::PRINCIPAL_MAIN,
        "t0",
        &delegations,
        &[],
        &[],
    );

    let snap = store.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let opened = snap
        .iter()
        .find(|e| e.kind == symbio_core::EVENT_COMMITMENT_OFFERED)
        .expect("立约格");
    assert_eq!(
        opened.payload.get("from").and_then(|v| v.as_str()),
        Some(symbio_core::PRINCIPAL_PEER),
        "承诺方 = 跑轮的主体（对等体）"
    );
    assert_eq!(
        opened.payload.get("to").and_then(|v| v.as_str()),
        Some(symbio_core::PRINCIPAL_MAIN),
        "承诺对象 = 来源主体（传入的数据，不是写死的常量）"
    );
}

/// 违约必须可观测（S08 §5）：`broken` 带 `why`，并**宣告**给承诺对象——
/// 宣告仍是同一份事实源里的一格，不是新通道（S08 §2「通信 = 没有直连」）。
#[test]
fn breached_delegation_is_declared_to_the_counterparty() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
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
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &delegations,
        &[],
        &[],
    );

    let snap = store.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));

    let broken = snap
        .iter()
        .find(|e| e.kind == symbio_core::EVENT_COMMITMENT_BROKEN)
        .expect("违约收束必须成格");
    assert_eq!(
        broken.payload.get("why").and_then(|v| v.as_str()),
        Some("子会话中途失败"),
        "违约必须带 why（可观测，S08 §5）"
    );

    let asserted = snap
        .iter()
        .find(|e| e.kind == symbio_core::EVENT_COMMITMENT_ASSERTED)
        .expect("违约宣告必须成格");
    assert_eq!(asserted.entity, Entity::Commitment);
    let statement = asserted
        .payload
        .get("statement")
        .and_then(|v| v.as_str())
        .expect("宣告载荷带原话");
    assert!(
        statement.contains(symbio_core::PRINCIPAL_USER)
            && statement.contains("子会话中途失败"),
        "宣告 = 把这次违约告知承诺对象：{statement}"
    );
    assert_eq!(
        asserted.produced_by,
        Some(user_seq),
        "宣告同样锚在本轮开口上"
    );
}

/// `Break` 的理由落成 `control × opened`，溯源锚是本轮开口；`breaks` 为空 ⇒ 一格都不落。
///
/// 这是 [roadmap/S09 §6](../../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断") 的两条
/// **相反**验收：验收 1（未授权 ⇒ 拒绝且不产生事件）与验收 2（预算耗尽 ⇒ 必须产出
/// 熔断事件）。两条必须由**同一个出参**分别触发——有理由就落、没理由就不落；
/// 「反正都落一条」违反 1，「反正都不落」违反 2。
#[test]
fn break_reason_lands_as_a_control_event_and_empty_stays_silent() {
    // ── 有理由：落格 ────────────────────────────────────────────────────
    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
        0,
        "让它跑一遍",
        7,
    );
    record_derived(
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &[],
        &[],
        &["budget-exhausted"],
    );

    let snap = store.range(Seq::new(0));
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

    // ── 没理由（`Refuse` 或未触发）：一格都不落 ────────────────────────
    let quiet = EventStore::new();
    let (snapshot, user_seq) = seed_turn(&quiet, symbio_core::PRINCIPAL_MAIN, 0, "问", 1);
    record_derived(
        "derived-test",
        &quiet,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &[],
        &[],
        &[],
    );
    let snap = quiet.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    assert!(
        !snap.iter().any(|e| e.entity == Entity::Control),
        "没有熔断理由 ⇒ 不产生任何事件（S09 §6 验收 1）"
    );
}

/// 记忆三段（步 11 编码 / 步 12 检索锚 / 步 13 巩固）全部在同一份 WAL 里可读，
/// 且 `check_all` 全绿（溯源 100%）。
///
/// 端到端接线由 `turn_exec.test.rs::full_turn_lands_memory_and_learning_facts` 带反向
/// 自检覆盖；本用例钉的是**巩固要用刷新后的快照**——步 11 刚写下的那条必须对步 13
/// 可见（先巩固后编码会让第 4 条永远等不到代数 1）。
#[test]
fn record_learning_wires_all_three_memory_steps() {
    let store = EventStore::new();

    // 轮 0–3：每轮说一句、四句互不相同 ⇒ 第 4 轮收束时活记忆到 4 条 ⇒ 巩固触发。
    for i in 0..4u64 {
        let (snapshot, user_seq) = seed_turn(
            &store,
            symbio_core::PRINCIPAL_MAIN,
            i,
            &format!("第 {i} 条约定"),
            i,
        );
        record_learning(
            "derived-test",
            &store,
            &snapshot,
            i,
            user_seq,
            &format!("第 {i} 条约定"),
            symbio_core::PRINCIPAL_MAIN,
            &format!("t{i}"),
            Some("答"),
            None,
            &[],
            false,
            symbio_core::clock_now_ms(),
        );
    }

    let snap = store.range(Seq::new(0));
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED && e.event_id.starts_with("v2m-"))
            .count(),
        4,
        "步 11：每轮各编码一条经验"
    );
    // 步 11b：判断编码同文去重——四轮收束都是同一句「答」，
    // 跨标签同文也只记一条（`contains_content` 比的是全部记忆）。
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_MEMORY_ENCODED && e.event_id.starts_with("v2j-"))
            .count(),
        1,
        "步 11b：判断每轮各编一条，同文去重（四轮同一句 ⇒ 只一条）"
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
    let view = recall(symbio_core::PRINCIPAL_USER, None, None)
        .apply(&snap, i64::MAX, Budget::generous())
        .value;
    assert!(!view.entries.is_empty(), "还有活记忆可召回");

    let (snapshot, user_seq) =
        seed_turn(&store, symbio_core::PRINCIPAL_MAIN, 4, "第五句", 44);
    record_learning(
        "derived-test",
        &store,
        &snapshot,
        4,
        user_seq,
        "第五句",
        symbio_core::PRINCIPAL_MAIN,
        "t4",
        Some("答"),
        Some(&view),
        &[],
        false,
        symbio_core::clock_now_ms(),
    );

    let snap = store.range(Seq::new(0));
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
}

// ── 淘汰回路（S13 §3，）：轮末声誉裁决 ──────────────────────────────

/// 一轮**违约**的委托 ⇒ 声誉 −1 ⇒ 轮末写一条 `verdict.asserted` 收回 `assign.work`。
///
/// 钉三件事：① 判据读的是**同轮刚落格**的违约（顺序判据——`record_elimination`
/// 必须排在 `record_derived` 之后，否则判据滞后一个轮次）；② 事件的作者是仲裁者
/// 而不是被淘汰主体；③ 载荷带 score / threshold，审计方看得见当时值。
#[test]
fn a_broken_delegation_this_turn_triggers_the_elimination_verdict() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
        0,
        "交给子智能体",
        50,
    );
    let delegations = [Delegation {
        id: "call-1".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: false,
        why: "子智能体没接手".to_string(),
    }];
    record_derived(
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &delegations,
        &[],
        &[],
    );

    // 轮末淘汰步（顺序即判据：排在承诺落格之后）。
    record_elimination(&store, 0, user_seq);

    let snap = store.range(Seq::new(0));
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    let verdict = snap
        .iter()
        .find(|e| e.kind == EVENT_VERDICT_ASSERTED)
        .expect("低于阈值必须落淘汰裁决");
    assert_eq!(verdict.entity, Entity::Verdict);
    assert_eq!(verdict.verb, Verb::Asserted);
    assert_eq!(
        verdict.actor,
        symbio_core::PRINCIPAL_ARBITER,
        "裁决者是仲裁者，不是被淘汰主体"
    );
    assert_eq!(
        verdict.produced_by,
        Some(user_seq),
        "裁决同锚在本轮开口（I2）"
    );
    let p = &verdict.payload;
    assert_eq!(p["subject"], symbio_core::PRINCIPAL_MAIN);
    assert_eq!(p["capability"], "assign.work");
    assert_eq!(p["change"], "revoke");
    assert_eq!(p["score"], -1, "判据值随事件落盘，审计方看得见");
    assert!(p["why"].as_str().is_some_and(|w| !w.is_empty()));
}

/// 反向：**守约**的委托 ⇒ 声誉 +1 ⇒ 不淘汰（阈值判据不是恒真）。
#[test]
fn a_kept_delegation_never_eliminates() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
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
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &delegations,
        &[],
        &[],
    );
    record_elimination(&store, 0, user_seq);

    let snap = store.range(Seq::new(0));
    assert!(
        !snap.iter().any(|e| e.kind == EVENT_VERDICT_ASSERTED),
        "守约 ⇒ 声誉 +1 ⇒ 一条裁决都不该有"
    );
}

/// 幂等：同一条撤销不重复落格（无既有 verdict 的主体才写）。
///
/// 事件号带溯源锚、重复写只会撞幂等键被存储**静默**丢掉——「审计面少一条」
/// 正是要禁的静默失效，故判据必须在写之前就问覆盖层。
#[test]
fn elimination_does_not_write_a_second_verdict() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
        0,
        "交给子智能体",
        50,
    );
    let delegations = [Delegation {
        id: "call-1".to_string(),
        promise: "让 reviewer 复查这段".to_string(),
        ok: false,
        why: "子智能体没接手".to_string(),
    }];
    record_derived(
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &delegations,
        &[],
        &[],
    );
    record_elimination(&store, 0, user_seq);
    // 第二轮再判一次（覆盖层里已经有那条撤销了）。
    record_elimination(&store, 1, user_seq);

    let snap = store.range(Seq::new(0));
    assert_eq!(
        snap.iter()
            .filter(|e| e.kind == EVENT_VERDICT_ASSERTED)
            .count(),
        1,
        "同一条撤销只落一次格（审计面不缺、幂等键不撞）"
    );
}

/// 范围边界：只淘汰**持 `assign.work` 的登记主体**——仅持 `produce.artifact` 的
/// 对等主体（`agent:peer-b` 类）不在机制内（它没有「派活」这个权威动作可收回）。
#[test]
fn a_principal_without_assign_work_is_never_eliminated() {
    use symbio_core::facts::Delegation;

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_PEER,
        0,
        "对等主体跑一轮",
        50,
    );
    let delegations = [Delegation {
        id: "call-peer-1".to_string(),
        promise: "把结果交给主智能体".to_string(),
        ok: false,
        why: "上游没给数据".to_string(),
    }];
    record_derived(
        "derived-test",
        &store,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_PEER,
        symbio_core::PRINCIPAL_MAIN,
        "t0",
        &delegations,
        &[],
        &[],
    );
    record_elimination(&store, 0, user_seq);

    let snap = store.range(Seq::new(0));
    assert!(
        !snap.iter().any(|e| e.kind == EVENT_VERDICT_ASSERTED),
        "对等主体不持 assign.work ⇒ 淘汰机制不覆盖它（stage 字面边界）"
    );
}

// ── 三族互不连累 ────────────────────────────────────────────────────────

/// 承诺写不进去时，任务表与熔断**照样写**。
///
/// 三族共用同一个锚点（`user_seq`），但彼此没有数据依赖——一族写失败就让另外两族
/// 的事实一起消失，是 `?` 的副作用，不是设计意图。此前的实现正是这样：承诺段用
/// `map_err(..)?`，于是第一个 `append` 失败后函数直接返回，任务表与熔断事件都没写。
///
/// 反向守卫：把承诺段改回 `?`，本用例立刻变红（`kind == "task.*"` 与熔断格都找不到）。
#[test]
fn commitment_failure_does_not_block_task_or_breaker() {
    use symbio_core::facts::{Delegation, TaskDeclaration, TaskItem, TaskStatus};

    /// 拒绝一切 `commitment.*` 事实、其余照常落库的写口。
    struct CommitmentsRefuse(EventStore);

    impl Store for CommitmentsRefuse {
        type Event = Event;
        fn append(&self, event: Event) -> Result<Seq, symbio_core::AppendError> {
            if event.kind.starts_with("commitment.") {
                return Err(symbio_core::AppendError::Duplicate);
            }
            self.0.append(event)
        }
        fn range(&self, from: Seq) -> Vec<Event> {
            self.0.range(from)
        }
        fn head(&self) -> Seq {
            self.0.head()
        }
    }

    let store = EventStore::new();
    let (snapshot, user_seq) = seed_turn(
        &store,
        symbio_core::PRINCIPAL_MAIN,
        0,
        "交给子智能体",
        50,
    );
    let refusing = CommitmentsRefuse(store);

    record_derived(
        "derived-test",
        &refusing,
        &snapshot,
        0,
        user_seq,
        symbio_core::PRINCIPAL_MAIN,
        symbio_core::PRINCIPAL_USER,
        "t0",
        &[Delegation {
            id: "call-1".to_string(),
            promise: "让 reviewer 复查这段".to_string(),
            ok: true,
            why: String::new(),
        }],
        &[TaskDeclaration {
            items: vec![TaskItem {
                id: "t1".to_string(),
                goal: "复查上下文收敛".to_string(),
                depends_on: vec![],
                status: TaskStatus::default(),
            }],
        }],
        &["成本超预算"],
    );

    let snap = refusing.range(Seq::new(0));
    // 断言一：承诺确实一条都没落（注入生效——否则本用例测的是"全成功"的平凡情形）。
    assert!(
        !snap.iter().any(|e| e.kind.starts_with("commitment.")),
        "注入没生效：承诺事件不该写进去"
    );
    // 断言二：另两族照常落库。
    assert!(
        snap.iter().any(|e| e.kind.starts_with("task.")),
        "承诺写失败后任务表被连累了——三族之间没有依赖，不该互相打断"
    );
    // 熔断格按 `Entity::Control` 认，不按 kind 字符串认：`EVENT_CONTROL_OPENED` 的
    // 字面量不住公开面（`event` 模块是私有的），而本用例判的是「那一格写没写进去」，
    // 不是它的 kind 拼写。
    assert!(
        snap.iter().any(|e| e.entity == Entity::Control),
        "承诺写失败后熔断格被连累了"
    );
}
