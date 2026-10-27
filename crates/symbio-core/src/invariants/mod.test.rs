//! `invariants` 单测 —— 每条检查都要证明「它红得起来」。
//!
//! 反向用例与本体的关系：正向用例证明检查**在违规时报警**，反向用例证明
//! 合规序列**不报警**（检查不是永远红的状态机）。两组都绿，检查才存在。

use super::*;
use crate::event::kernel_vocab;
use crate::event::{Entity, Event, EventEnvelope, Verb};
use crate::store::{EventStore, Store};
use crate::ClosureReason;

/// 造一条 pending 事件（未入库，无 seq）。
///
/// **不带溯源边**：根事件（`user.message`）本来就没有上游。带边的用例显式
/// `.with_produced_by(..)`——此前这里默认填 0，于是"首条事件指向自己"被当成合规，
/// 而 [`provenance_dangling`] 一上线，那个默认值就是一处假绿的来源。
fn pending(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, entity, verb, turn, "main")
}

/// 造一批**已入库**事件（seq 由 Store 分配 ⇒ 天然单调）。
fn stored(events: Vec<Event>) -> Vec<Event> {
    let store = EventStore::new();
    for e in events {
        store.append(e).expect("首插必成");
    }
    store.range(crate::event::Seq::new(0))
}

// ── C1：seq 单调 ───────────────────────────────────────────────────────

#[test]
fn seq_monotonic_passes_on_store_output() {
    let events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("e2", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    assert!(
        seq_monotonic(&events).is_empty(),
        "Store 产出的序列天然单调：{:?}",
        seq_monotonic(&events)
    );
}

#[test]
fn seq_gap_is_detected() {
    let mut events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    // 模拟「中间丢了一条」：删除 seq=1 后，后一条应接 seq=1 却是 seq=2。
    events.remove(1);
    // 重新造：seq 0 与 seq 2 相邻（跳号）。
    let mut e2 = pending("e2", "user.message", Entity::Turn, Verb::Opened, 2);
    e2.assign_seq(crate::event::Seq::new(2));
    events.push(e2);
    let bad = seq_monotonic(&events);
    assert_eq!(bad.len(), 1, "跳号必须被看见");
    assert_eq!(bad[0].event_id, "e2");
    assert!(bad[0].why.contains("严格单调"), "{}", bad[0].why);
}

#[test]
fn pending_events_are_skipped_by_seq_check() {
    // pending（未入库）事件没有 seq，不参与单调性检查——它们还不是事实。
    let events = vec![pending("p0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(seq_monotonic(&events).is_empty());
}

// ── C2 / N3：每 turn 至多 1 条 final ───────────────────────────────────

#[test]
fn one_final_per_turn_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    assert!(final_unique_per_turn(kernel_vocab(), &events).is_empty());
}

#[test]
fn two_finals_in_one_turn_is_detected() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        // 同一 turn 的第二条 final——转写收敛性被破坏。
        pending("f0b", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
    ]);
    let bad = final_unique_per_turn(kernel_vocab(), &events);
    assert_eq!(bad.len(), 1, "一轮两条 final 必须被看见");
    assert_eq!(bad[0].event_id, "f0b");
    assert!(
        bad[0].why.contains("f0"),
        "违规要指出已存在的那条：{}",
        bad[0].why
    );
}

#[test]
fn closed_tasks_do_not_count_as_final() {
    // `task.completed` 也是 closed，但不是发言——N3 只管 turn 的发言收敛。
    let events = stored(vec![
        pending("t0", "task.completed", Entity::Task, Verb::Closed, 0),
        pending("t1", "task.completed", Entity::Task, Verb::Closed, 0),
    ]);
    assert!(final_unique_per_turn(kernel_vocab(), &events).is_empty());
}

// ── C2 对偶：每 turn 至多 1 条开轮（`u-{turn}`）────────────────────────

#[test]
fn one_open_per_turn_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    assert!(
        open_unique_per_turn(&events).is_empty(),
        "每轮只开一次 ⇒ 不报警：{:?}",
        open_unique_per_turn(&events)
    );
}

#[test]
fn two_opens_in_one_turn_is_detected() {
    // 同一个 `u-{turn}` 落两回——单写者被绕过的痕迹（plan/11 批 0）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("u0-again", "user.message", Entity::Turn, Verb::Opened, 0),
    ]);
    let bad = open_unique_per_turn(&events);
    assert_eq!(bad.len(), 1, "一轮两条开轮必须被看见");
    assert_eq!(bad[0].event_id, "u0-again", "违规锚在**后落**的那条上");
    assert!(
        bad[0].why.contains("u0"),
        "违规要指出已存在的那条：{}",
        bad[0].why
    );
}

/// 开轮那一格上的**任何**名字都算开轮（DE-024 D-203 裁决一）。
///
/// 判据一旦在这里额外比一次名字，写侧 `is_turn_open`（只认格子）与读侧本检查就是两
/// 条尺子，而后果是单向的：一个新名字落进开轮格 ⇒ 读侧静默不拦 ⇒ 同一个 `u-{turn}`
/// 成了两条输入、兜底率的分母虚增，全绿。格子语义不问名字。
#[test]
fn any_kind_on_the_opened_coordinate_opens_the_turn() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("x0", "turn.reopened", Entity::Turn, Verb::Opened, 0),
    ]);
    let bad = open_unique_per_turn(&events);
    assert_eq!(
        bad.len(),
        1,
        "同格的第二个名字同样是开轮，必须被看见：{bad:?}"
    );
    assert_eq!(bad[0].event_id, "x0", "违规锚在**后落**的那条上");
}

// ── C3 / N5：断言类事件必须带溯源 ──────────────────────────────────────
#[test]
fn asserted_with_provenance_passes() {
    let events = stored(vec![pending(
        "v0",
        "classify.verdict",
        Entity::Verdict,
        Verb::Asserted,
        0,
    )
    .with_produced_by(0)]);
    assert!(produced_by_coverage(&events).is_empty());
}

#[test]
fn asserted_without_provenance_is_detected() {
    let no_source = pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0);
    // 显式去掉溯源（pending 默认带 0，这里造一条真正的裸断言）。
    let no_source = Event {
        produced_by: None,
        ..no_source
    };
    let events = stored(vec![no_source]);
    let bad = produced_by_coverage(&events);
    assert_eq!(bad.len(), 1, "裸断言必须被看见（I2 无溯源不声明）");
    assert_eq!(bad[0].event_id, "v0");
}

#[test]
fn non_asserted_events_need_no_provenance() {
    // 普通过程事件（opened 等）不是声明，不要求 produced_by——
    // S1 起收束类（chat.assistant.final/fallback）例外，见下。
    let events = vec![pending("u0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(produced_by_coverage(&events).is_empty());
}

#[test]
fn reply_without_provenance_is_detected() {
    // S01 §5：回复必须有 produced_by——用户收到的话必须可追溯。
    let bare = Event {
        produced_by: None,
        ..pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0)
    };
    let bad = produced_by_coverage(std::slice::from_ref(&bare));
    assert_eq!(bad.len(), 1, "无溯源的答复必须被看见（I2）");
    assert!(bad[0].why.contains("收束"), "{}", bad[0].why);
}

#[test]
fn fallback_without_provenance_is_detected() {
    // 兜底也是一条普通事件——同样逃不出 I2（roadmap/S01 §2 的关键设计）。
    let bare = Event {
        produced_by: None,
        ..pending(
            "fb0",
            "chat.assistant.fallback",
            Entity::Turn,
            Verb::Closed,
            0,
        )
    };
    assert_eq!(produced_by_coverage(std::slice::from_ref(&bare)).len(), 1);
}

#[test]
fn reply_with_provenance_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0).with_produced_by(0),
    ]);
    assert!(produced_by_coverage(&events).is_empty());
}

// ── C3 对偶：溯源不悬空 ────────────────────────────────────────────────
//
// 这四条钉的是「有没有」之外的另一半：「指向谁」。
// 每一条都能在关掉 [`provenance_dangling`] 之后变绿——那正是它存在的证明。

/// 造一条**已入库**事件，`seq` 由测试指定（用来造切片与空洞）。
fn at_seq(seq: u64, id: &str, produced_by: Option<u64>) -> Event {
    let mut e = Event::pending(
        id,
        "classify.verdict",
        Entity::Verdict,
        Verb::Asserted,
        0,
        "main",
    );
    e.seq = Some(crate::event::Seq::new(seq));
    e.produced_by = produced_by;
    e
}

#[test]
fn provenance_to_earlier_existing_event_passes() {
    let events = vec![at_seq(0, "e0", None), at_seq(1, "e1", Some(0))];
    assert!(
        provenance_dangling(&events, false).is_empty(),
        "指向更早且存在的事件 = 合规"
    );
}

#[test]
fn provenance_pointing_forward_is_detected() {
    // 指向一条**更晚**的事件：边把因果倒过来了，而日志读起来毫无异样。
    let events = vec![
        at_seq(0, "e0", Some(2)),
        at_seq(1, "e1", None),
        at_seq(2, "e2", None),
    ];
    let bad = provenance_dangling(&events, false);
    assert_eq!(bad.len(), 1, "前向边必须被看见");
    assert_eq!(bad[0].event_id, "e0");
    assert!(bad[0].why.contains("不小于"), "{}", bad[0].why);
}

#[test]
fn provenance_pointing_at_itself_is_detected() {
    // `produced_by == seq` 是前向边的极端情形：自己产生自己。
    let events = vec![at_seq(3, "e3", Some(3))];
    let bad = provenance_dangling(&events, false);
    assert_eq!(bad.len(), 1, "自指也必须被看见");
}

#[test]
fn provenance_into_a_hole_is_dangling() {
    // 切片内**有空洞**：seq 5 与 7 在，6 不在。7 指向 6 ⇒ 悬空。
    // 这一条与"切片首部"不同：它不是"更早但在片外"，是"该在片里却不在"。
    let events = vec![at_seq(5, "e5", None), at_seq(7, "e7", Some(6))];
    let bad = provenance_dangling(&events, true);
    assert_eq!(bad.len(), 1, "空洞里的溯源边必须被看见（宽限也放不过它）");
    assert!(bad[0].why.contains("悬空"), "{}", bad[0].why);
}

#[test]
fn provenance_across_the_slice_head_is_graced() {
    // 切片从 seq 6 起，6 指向片外的 5：**合法**，只是本切片看不到上游。
    let events = vec![at_seq(6, "e6", Some(5))];
    assert!(
        provenance_dangling(&events, true).is_empty(),
        "读侧宽限档：跨片边不假红"
    );
    assert_eq!(
        provenance_dangling(&events, false).len(),
        1,
        "严判档（故障注入 / 整份日志）：跨片边照样要看见——宽限是形参，不是判据"
    );
}

#[test]
fn provenance_beyond_the_slice_tail_is_graced_too() {
    // 头尾**对称**：切片尾部事件的产物指向一条更晚的事实，本切片看不到它。
    // 只宽限头部等于假设"切片总是从头切"，而读侧窗口两头都会切。
    let events = vec![at_seq(0, "e0", None), at_seq(1, "e1", Some(9))];
    assert!(
        provenance_dangling(&events, true).is_empty(),
        "尾部跨片边同样不假红（对称宽限）"
    );
    assert_eq!(
        provenance_dangling(&events, false).len(),
        1,
        "严判档照样要看见"
    );
}

#[test]
fn pending_events_are_skipped_by_dangling_check() {
    // 未入库事件还没有 seq，溯源边尚无意义（与 `seq_monotonic` 同口径）。
    let pending_only = vec![Event::pending(
        "p0",
        "classify.verdict",
        Entity::Verdict,
        Verb::Asserted,
        0,
        "main",
    )
    .with_produced_by(999)];
    assert!(provenance_dangling(&pending_only, false).is_empty());
}

// ── C6：停机原因闭集 ───────────────────────────────────────────────────

/// 造一条带 `reason` 的收束格。
fn closed(id: &str, kind: &str, reason: Option<&str>) -> Event {
    let payload = match reason {
        Some(r) => serde_json::json!({ "reason": r }),
        None => serde_json::json!({}),
    };
    Event::pending(id, kind, Entity::Turn, Verb::Closed, 0, "main").with_payload(payload)
}

#[test]
fn closure_reason_in_the_closed_set_passes() {
    for r in ClosureReason::ALL {
        let events = vec![closed("f0", "chat.assistant.final", Some(r.as_str()))];
        assert!(
            closure_reason_closed(kernel_vocab(), &events, true).is_empty(),
            "{r:?} 是闭集里的合法取值，不该红"
        );
    }
}

#[test]
fn closure_reason_missing_is_reported_when_not_graced() {
    let events = vec![closed("f0", "chat.assistant.final", None)];
    let strict = closure_reason_closed(kernel_vocab(), &events, false);
    assert_eq!(strict.len(), 1, "严判档：缺 `reason` 必须红");
    assert!(
        strict[0].why.contains("缺 `reason`"),
        "要说清缺的是什么：{}",
        strict[0].why
    );
    assert!(
        closure_reason_closed(kernel_vocab(), &events, true).is_empty(),
        "读侧宽限档：接线前落库的历史收束格没有 `reason`，首日不假红"
    );
}

#[test]
fn closure_reason_outside_the_closed_set_is_reported_either_way() {
    // 越界**不放宽**：写了一个不在闭集里的值，从来不是历史问题。
    let events = vec![closed("f0", "chat.assistant.final", Some("timeout"))];
    for graced in [true, false] {
        let bad = closure_reason_closed(kernel_vocab(), &events, graced);
        assert_eq!(bad.len(), 1, "越界在两种档位下都要红（graced={graced}）");
        assert!(
            bad[0].why.contains("timeout") && bad[0].why.contains("闭集"),
            "要把越界的值与合法取值一起给出：{}",
            bad[0].why
        );
    }
}

#[test]
fn grace_ends_at_the_first_stop_cell_that_carries_a_reason() {
    // 口径 ③：宽限的分界由日志自己说——第一条带 `reason` 的停机格之前是历史段。
    let events = vec![
        closed("f0", "chat.assistant.final", None),
        closed("f1", "chat.assistant.final", Some("completed")),
    ];
    assert!(
        closure_reason_closed(kernel_vocab(), &events, true).is_empty(),
        "分界**之前**的缺口算历史归档，仍要放宽"
    );
    assert_eq!(
        closure_reason_closed(kernel_vocab(), &events, false).len(),
        1,
        "严判档不看分界：历史段的缺口一样报"
    );
}

#[test]
fn a_new_gap_after_the_boundary_is_red_under_the_graced_read() {
    // 这条是「整段放宽」与「按分界放宽」的实测差异点：
    // 写方已经开始写 `reason`，之后漏一条就不可能再是历史造成的。
    let events = vec![
        closed("f0", "chat.assistant.final", Some("completed")),
        closed("f1", "chat.assistant.final", None),
    ];
    let bad = closure_reason_closed(kernel_vocab(), &events, true);
    assert_eq!(
        bad.len(),
        1,
        "分界**之后**缺 `reason` 必须红（旧口径下这条永远绿）"
    );
    assert!(
        bad[0].why.contains("缺 `reason`"),
        "要指出缺的是 `reason`：{}",
        bad[0].why
    );
}

#[test]
fn an_all_historical_log_stays_green_under_the_graced_read() {
    // 整条日志一个 `reason` 都没有 ⇒ 没有分界，与旧「整段放宽」逐字等价。
    let events = vec![
        closed("f0", "chat.assistant.final", None),
        closed("f1", "chat.assistant.fallback", None),
        closed("f2", "chat.assistant.final", None),
    ];
    assert!(
        closure_reason_closed(kernel_vocab(), &events, true).is_empty(),
        "纯历史存档不该飘红"
    );
    assert_eq!(
        closure_reason_closed(kernel_vocab(), &events, false).len(),
        3,
        "严判档仍逐条报"
    );
}

#[test]
fn closure_reason_covers_both_closure_cells_and_the_denial_fact() {
    // `fallback` 是另一个格名，`authz.denied` 根本不是收束格——三处都问同一个问题。
    let events = vec![
        closed("f0", "chat.assistant.final", None),
        closed("fb0", "chat.assistant.fallback", None),
        Event::pending(
            "d0",
            "authz.denied",
            Entity::Turn,
            Verb::Asserted,
            0,
            "main",
        ),
    ];
    assert_eq!(
        closure_reason_closed(kernel_vocab(), &events, false).len(),
        3,
        "两种收束格 + 否定事实，缺 `reason` 一个都逃不掉"
    );
}

#[test]
fn closure_reason_denied_on_a_denial_fact_passes() {
    let e = Event::pending(
        "d0",
        "authz.denied",
        Entity::Turn,
        Verb::Asserted,
        0,
        "main",
    )
    .with_payload(serde_json::json!({ "reason": ClosureReason::Denied.as_str() }));
    assert!(closure_reason_closed(kernel_vocab(), &[e], false).is_empty());
}

#[test]
fn non_closure_cells_are_not_asked_for_a_reason() {
    // `turn × progressed`（等待用户）**没有停**，不该被问「为什么停」。
    let events = vec![Event::pending(
        "p0",
        "chat.assistant.progress",
        Entity::Turn,
        Verb::Progressed,
        0,
        "main",
    )];
    assert!(
        closure_reason_closed(kernel_vocab(), &events, false).is_empty(),
        "进展格不是停机格"
    );
}

#[test]
fn an_artifact_is_settled_but_not_a_stop_so_it_is_not_asked_for_a_reason() {
    // `closes_turn`（I3「结算了吗」）把产物格算进去；C6 问的是「为什么停」，
    // 产物落格后轮还在走，生产写方永远写不出这条 reason。把两把尺合一，
    // 判据就变成向生产要一个它给不出的字段（`artifact.added 缺 reason` 的飘红）。
    let events = vec![
        closed("f0", "chat.assistant.final", Some("completed")),
        Event::pending(
            "a0",
            "artifact.added",
            Entity::Artifact,
            Verb::Asserted,
            0,
            "main",
        ),
    ];
    assert!(
        closure_reason_closed(kernel_vocab(), &events, false).is_empty(),
        "产物格不是停机格：{:?}",
        closure_reason_closed(kernel_vocab(), &events, false)
    );
}

// ── check_all：九条合跑 ────────────────────────────────────────────────

#[test]
fn check_all_on_clean_sequence_is_empty() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        // 收束类**必须有**溯源（I2）：指向开轮那条。
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0).with_produced_by(0),
        pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0).with_produced_by(0),
    ]);
    assert!(
        check_all(kernel_vocab(), &events).is_empty(),
        "合规序列九查全绿"
    );
}

// ── 故障注入用例集（S1 步骤 4：超时 / 乱序 / 重复 final / 无溯源，4 类违规全被捕获）────
//
// [plan/04 §3](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 第 4 步的产物：每类故障注入
// 事件序列，断言**对应的检查必须红**。反向用例（合规序列不报警）在上方各检查区。

#[test]
fn fault_timeout_turn_never_settles_is_caught() {
    // 超时 = 生成失败且什么都没写——turn 开了却没有 final/fallback。
    let events = stored(vec![pending(
        "u0",
        "user.message",
        Entity::Turn,
        Verb::Opened,
        0,
    )]);
    // 严判档（`false`）：故障注入要的是"开了没关就是红"。
    let bad = unresolved_turns(&events, false);
    assert_eq!(bad.len(), 1, "静默中断必须被看见（I3：兜底必须产生事件）");
    assert!(bad[0].why.contains("未收束"), "{}", bad[0].why);
    // 宽限档（`true`）：切片尾部这一轮可能只是**还在生成**——纯函数无从分辨，
    // 放行（读侧口径，见 `check_all`）。两条判据并存，不是谁覆盖谁。
    assert!(
        check_all(kernel_vocab(), &events).is_empty(),
        "读侧不该把在途尾轮报成静默中断：{:?}",
        check_all(kernel_vocab(), &events)
    );
}

#[test]
fn fault_timeout_avoided_by_fallback_event() {
    // 反向：超时后**写了** fallback → turn 收束，检查放行（兜底是普通事件）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending(
            "fb0",
            "chat.assistant.fallback",
            Entity::Turn,
            Verb::Closed,
            0,
        ),
    ]);
    assert!(unresolved_turns(&events, false).is_empty());
}

#[test]
fn fault_timeout_avoided_by_artifact_event() {
    // 反向（[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")）：只落了**产物格**的轮
    // 同样算收束——工具结果是说给用户的话，没有发言权的主体（对等主体）以它收束。
    // 没有这条，对等主体每跑一轮都永久留一条违规，e2e 的「不变量清单为空」无从谈起。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending(
            "a0",
            crate::EVENT_ARTIFACT_ADDED,
            Entity::Artifact,
            Verb::Asserted,
            0,
        )
        .with_produced_by(0),
    ]);
    assert!(
        unresolved_turns(&events, false).is_empty(),
        "产物收束 ⇒ 放行：{:?}",
        unresolved_turns(&events, false)
    );
    // 边界：**别的轮**的产物不算本轮的收束（按 turn 号配对，与 final 同一纪律）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending(
            "a1",
            crate::EVENT_ARTIFACT_ADDED,
            Entity::Artifact,
            Verb::Asserted,
            1,
        )
        .with_produced_by(0),
    ]);
    assert_eq!(
        unresolved_turns(&events, false)
            .iter()
            .map(|v| v.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["u0"],
        "turn 0 没有自己的产物，仍要报；turn 1 被自己的产物合上"
    );
}

#[test]
fn fault_out_of_order_seq_regression_is_caught() {
    // 乱序 = seq 回退（3 之后又来 1）——绕过唯一写入口的典型痕迹。
    let mut e3 = pending("e3", "task.progress", Entity::Task, Verb::Progressed, 0);
    e3 = Event {
        seq: Some(crate::event::Seq::new(3)),
        ..e3
    };
    let mut e1 = pending("e1", "task.progress", Entity::Task, Verb::Progressed, 1);
    e1 = Event {
        seq: Some(crate::event::Seq::new(1)),
        ..e1
    };
    let bad = seq_monotonic(&[e3, e1]);
    // 两条都被看见：seq 3 处应为 0（开头顶格），seq 1 处应为 4（回退）。
    assert_eq!(bad.len(), 2, "乱序注入的两处异常都必须被看见：{:?}", bad);
    assert_eq!(bad[1].event_id, "e1");
}

#[test]
fn fault_double_final_is_caught_by_check_all() {
    // 重复 final 经 check_all 合跑也必须红（CI 形态下无漏网）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
    ]);
    let violations = check_all(kernel_vocab(), &events);
    assert!(
        violations.iter().any(|v| v.why.contains("final")),
        "{:?}",
        violations
    );
}

#[test]
fn fault_double_open_is_caught_by_check_all() {
    // 重复开轮经 check_all 合跑也必须红，且是**唯一**那条（该轮正常收束 ⇒ 不牵连 C4）。
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("u0-again", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0).with_produced_by(0),
    ]);
    let violations = check_all(kernel_vocab(), &events);
    assert_eq!(violations.len(), 1, "只有开轮这一条红：{violations:?}");
    assert!(violations[0].why.contains("开轮"), "{violations:?}");
}

#[test]
fn fault_missing_provenance_is_caught_by_check_all() {
    // 无溯源的断言经 check_all 合跑也必须红。
    let bare = Event {
        produced_by: None,
        ..pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0)
    };
    let events = stored(vec![bare]);
    assert!(
        check_all(kernel_vocab(), &events)
            .iter()
            .any(|v| v.why.contains("溯源")),
        "{:?}",
        check_all(kernel_vocab(), &events)
    );
}

#[test]
fn fault_budget_exceeded_is_caught() {
    // I3 记账：单事件耗时超主体预算必须被看见（超预算允许发生，但不容隐身）。
    let events =
        vec![pending("s0", "task.progress", Entity::Task, Verb::Progressed, 0).with_cost_ms(2_000)];
    let bad = budget_exceeded(&events, Some(1_500));
    assert_eq!(bad.len(), 1, "超预算必须被看见");
    assert!(bad[0].why.contains("2"), "{}", bad[0].why);
    // 反向：预算内不报警。
    let ok =
        vec![pending("s1", "task.progress", Entity::Task, Verb::Progressed, 0).with_cost_ms(1_499)];
    assert!(budget_exceeded(&ok, Some(1_500)).is_empty());
    // 宽限档：没给预算就无从判"超"——返回空，不是拿默认数硬比。
    assert!(budget_exceeded(&events, None).is_empty(), "无预算 ⇒ 不判");
}

// ── 读侧宽限（04 §3.1 批④：两条 I3 检查接进读出口前先立的口径）────────────

#[test]
fn read_side_graces_the_trailing_open_turn_but_flags_the_overtaken_one() {
    // 轮 0 开了没收束，轮 1 随后开轮并正常收束——会话已经往前走，轮 0 是确凿的
    // 静默中断（不是"还在生成"），读侧必须报。
    let overtaken = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    let bad = unresolved_turns(&overtaken, true);
    assert_eq!(bad.len(), 1, "被后续轮越过的未收束轮必须报");
    assert_eq!(bad[0].event_id, "u0", "违规锚在缺口本身（开轮那条）");
    assert!(
        check_all(kernel_vocab(), &overtaken)
            .iter()
            .any(|v| v.why.contains("未收束")),
        "经 check_all 也要红：{:?}",
        check_all(kernel_vocab(), &overtaken)
    );
}

#[test]
fn read_side_budget_follows_declared_tiers_and_skips_when_none_declared() {
    // 声明了 reflex 档（80ms）⇒ 预算有值，2000ms 的事件必须被看见。
    let declared = stored(vec![
        Event::pending("u0", "user.message", Entity::Turn, Verb::Opened, 0, "user")
            .with_payload(serde_json::json!({ "text": "问", "tier": "reflex" })),
        Event::pending(
            "s0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "session/autonomous",
        )
        .with_produced_by(0)
        .with_cost_ms(2_000),
    ]);
    assert_eq!(
        declared_budget_ms(kernel_vocab(), &declared),
        Some(80),
        "声明档位 ⇒ 预算 = 该档四层预算表"
    );
    assert!(
        check_all(kernel_vocab(), &declared)
            .iter()
            .any(|v| v.why.contains("超")),
        "读侧必须看见超预算：{:?}",
        check_all(kernel_vocab(), &declared)
    );

    // 一档都没声明 ⇒ `None` ⇒ 不判（没有预算就没有"超"）。
    let undeclared = stored(vec![pending(
        "s1",
        "task.progress",
        Entity::Task,
        Verb::Progressed,
        0,
    )
    .with_cost_ms(9_999_999)]);
    assert_eq!(declared_budget_ms(kernel_vocab(), &undeclared), None);
    assert!(
        check_all(kernel_vocab(), &undeclared).is_empty(),
        "未声明档位的切片首日不该红：{:?}",
        check_all(kernel_vocab(), &undeclared)
    );
}

#[test]
fn declared_budget_prefers_spec_numeric_and_falls_back_to_tier() {
    // 两种形态同一条事实、同一来源（开轮载荷）。判据：**数值优先、档位回退**，
    // 且回退只在缺数值时生效——两处形态至多一处被读（不是「取更宽的那个」）。

    // ① 数值优先：档位 reflex(80) 会判超，但载荷声明了规格派生的 5000ms，
    //    2000ms 的事件必须**不被**报——若读的是档位就会误红。
    let numeric_wins = stored(vec![
        Event::pending("u0", "user.message", Entity::Turn, Verb::Opened, 0, "user").with_payload(
            serde_json::json!({ "text": "问", "tier": "reflex", "budget_ms": 5_000 }),
        ),
        Event::pending(
            "s0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "session/autonomous",
        )
        .with_produced_by(0)
        .with_cost_ms(2_000),
    ]);
    assert_eq!(
        declared_budget_ms(kernel_vocab(), &numeric_wins),
        Some(5_000),
        "载荷带 budget_ms 数值 ⇒ 取数值，不取档位映射"
    );
    assert!(
        !check_all(kernel_vocab(), &numeric_wins)
            .iter()
            .any(|v| v.why.contains("超")),
        "数值优先：2000ms 未超 5000ms，不得报：{:?}",
        check_all(kernel_vocab(), &numeric_wins)
    );

    // ② 数值更严时照样红（证明它不是「只取更宽」，而是「数值说了算」）。
    let numeric_strict = stored(vec![
        Event::pending("u0", "user.message", Entity::Turn, Verb::Opened, 0, "user")
            .with_payload(serde_json::json!({ "text": "问", "tier": "deep", "budget_ms": 50 })),
        Event::pending(
            "s0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "session/autonomous",
        )
        .with_produced_by(0)
        .with_cost_ms(2_000),
    ]);
    assert_eq!(
        declared_budget_ms(kernel_vocab(), &numeric_strict),
        Some(50)
    );
    assert!(
        check_all(kernel_vocab(), &numeric_strict)
            .iter()
            .any(|v| v.why.contains("超")),
        "数值严于档位时也必须看见超预算：{:?}",
        check_all(kernel_vocab(), &numeric_strict)
    );

    // ③ 缺数值回退档位（向后兼容：接线前落的历史事件 / 旧 WAL）。
    let tier_fallback = stored(vec![Event::pending(
        "u0",
        "user.message",
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "问", "tier": "reflex" }))]);
    assert_eq!(
        declared_budget_ms(kernel_vocab(), &tier_fallback),
        Some(80),
        "无 budget_ms ⇒ 回退档位映射"
    );

    // ④ 两种形态都缺 ⇒ `None`（不判）。
    let neither = stored(vec![Event::pending(
        "u0",
        "user.message",
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "问" }))]);
    assert_eq!(declared_budget_ms(kernel_vocab(), &neither), None);
}

// ── D-203：判据读声明，不读名字 ─────────────────────────────────────────

/// 「什么算定稿发言」的答案在**词汇表**上，不在这个函数里（DE-024 D-203 裁决二）。
///
/// 一张新插件自己造的表：两个名字落同一格（`turn × closed`），只有其中一个声明了
/// `finalizes_turn`。判据必须数前者、放过后者——于是横向扩充加一个名字时，守卫跟着
/// 声明走，而不是跟着内核抄的那串字符走（那才是「加能力必改架构」的来源）。
#[test]
fn a_final_is_counted_because_it_declares_the_role_not_because_of_its_name() {
    const VOCAB: &[EventDecl] = &[
        EventDecl::with_roles(
            "fixture.my_final",
            (Entity::Turn, Verb::Closed),
            &[FactRole::FinalizesTurn],
        ),
        EventDecl::new("fixture.my_silent", (Entity::Turn, Verb::Closed)),
    ];
    let closed_at = |id: &str, kind: &str, turn: u64| {
        pending(id, kind, Entity::Turn, Verb::Closed, turn).with_produced_by(0)
    };

    // 声明了角色的名字：同轮两条 ⇒ 违规（正向，拦得住）。
    let dup = stored(vec![
        closed_at("f-1", "fixture.my_final", 3),
        closed_at("f-2", "fixture.my_final", 3),
    ]);
    let bad = final_unique_per_turn(VOCAB, &dup);
    assert_eq!(bad.len(), 1, "同轮第二条定稿必须被看见：{bad:?}");

    // 同格但没声明角色的名字：一条都不算定稿（反向，不越权拦）。
    let silent = stored(vec![
        closed_at("s-1", "fixture.my_silent", 3),
        closed_at("s-2", "fixture.my_silent", 3),
    ]);
    assert!(
        final_unique_per_turn(VOCAB, &silent).is_empty(),
        "没扮演角色的名字不是定稿，收束格之外的拦法不是这条判据"
    );

    // 同一批事件换一张**不认识这些名字**的表 ⇒ 判据空转（这正是装配口
    // `Gap::UnmannedRole` 与门禁 CS-008 要拦的形状，不是判据自己的错）。
    assert!(
        final_unique_per_turn(kernel_vocab(), &dup).is_empty(),
        "词汇表里没有的名字不扮演任何角色——角色来自表，不是来自猜"
    );
}

/// 停机事实的第二臂同样读角色（C6）：`authz.denied` 那一条的名字不出现在判据里。
#[test]
fn a_stop_fact_is_a_grid_cell_or_a_declared_role() {
    const VOCAB: &[EventDecl] = &[EventDecl::with_roles(
        "fixture.denied",
        (Entity::Turn, Verb::Asserted),
        &[FactRole::StopsTurn],
    )];
    let denied = pending("d-1", "fixture.denied", Entity::Turn, Verb::Asserted, 1);
    assert!(
        is_stop_fact(VOCAB, &denied),
        "声明了 stops_turn 的新名字就是停机事实"
    );
    // 收束格整格算，不看名字（第一臂与词汇无关）。
    assert!(is_stop_fact(
        VOCAB,
        &pending("c-1", "fixture.any", Entity::Turn, Verb::Closed, 1)
    ));
    // 既不在收束格、又没声明角色 ⇒ 不是停（产物格落在这里，与 `closes_turn` 分歧）。
    assert!(!is_stop_fact(
        VOCAB,
        &pending("a-1", "fixture.any", Entity::Artifact, Verb::Asserted, 1)
    ));
    assert!(
        !is_stop_fact(
            VOCAB,
            &pending("u-1", "fixture.other", Entity::Turn, Verb::Asserted, 1)
        ),
        "没声明角色的 `turn × asserted` 不是停机事实——角色表就是这一臂的唯一入口"
    );
}
