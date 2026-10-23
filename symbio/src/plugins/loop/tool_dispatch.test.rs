//! 工具端口三笔债的判据（ 步 4）：授权 / 派发 / 关账由**生产代码**写，
//! 行表按它们判欠不欠。
//!
//! ## 场景三（中止唤醒）：占位结果不是应答
//!
//! 一条 `sent` 之后只有两种收场：工具回了话（`response`），或执行层没跑到它
//! （`failed{why}`）。判据是「日志说的与发生的事一致」——批尾收口给没跑到的调用
//! 补的那条占位结果是**框架的交代**（给模型一句话、给前端一张卡），不是工具的
//! 应答；把它算成应答，中止的调用就在日志里写成「回来了」，`await-tools` 的
//! `failed` 出口永远等不到人（DE-015 §5 判据 6 的中止场景就是这么空的）。
//!
//! 反向的那一半同样在本文件钉住：工具**跑完而失败**（`error`）、闸门交回审批卡
//! （`needs_approval`）都是应答，落 `response`。「凡失败都算没跑到」会把真正回了
//! 话的调用写成没回——那是另一种失真，不是修正在第一种。
//!
//! ## 场景一（审批唤醒）：`authz.granted` 重新开账
//!
//! 授权动作落下 `authz.granted(port=tool)` 那一刻，`dispatch-tools` 必须**再次欠着**
//! ——「用户批了」在事实面上的全部含义就是「这笔派发重新成为一笔待还的账」。
//! 行层那几条合成用例（`loop_rows.test.rs`）对的是声明；这里对的是写方：
//! `ToolPort::authorize_calls` / `begin_calls` 真的产出那种载荷、真的换一笔债。
//!
//! 场景二（补充插话）不在这里：补充不产生任何端口事实，它的判据是「`await-tools`
//! 的 `until` 没有多一个出口」，归行表用例（`a_supplement_does_not_answer_the_in_flight_tool`）。
//!
//! 行不欠、幂等键这些引擎纪律已由 `port.test.rs` 覆盖，本文件只验「哪一种动作落
//! 哪一条事实、落了之后哪一笔债开或关」。

use serde_json::json;
use std::sync::Arc;

use super::super::tools::{ABORTED, NOT_EXECUTED};
use super::loop_rows;
use super::port;
use super::{DispatchOutcome, ToolPort, ToolPortCall, ToolPortPolicy, WakeIntent, WakeSpec};
use crate::symbio_core::{
    CapabilityRiskLevel, Event, EventStore, Seq, Store, EVENT_AUTHZ_DENIED, EVENT_AUTHZ_GRANTED,
    EVENT_REQUEST, EVENT_SENT,
};

/// 一轮的账：turn=1、溯源到用户格 seq=7、幕锚填占位 0（这三笔工具债不读时间视图）。
const TURN: u64 = 1;
const USER_SEQ: u64 = 7;

fn events(store: &EventStore) -> Vec<Event> {
    store.range(Seq::new(0))
}

fn kinds(store: &EventStore) -> Vec<String> {
    events(store).into_iter().map(|e| e.kind).collect()
}

fn payload_of(store: &EventStore, kind: &str) -> serde_json::Value {
    events(store)
        .into_iter()
        .find(|e| e.kind == kind)
        .map(|e| e.payload)
        .unwrap_or(serde_json::Value::Null)
}

/// 某一行此刻是否欠着这笔账（行表口径，不是内存计数）。
fn owes(row: &str, store: &EventStore) -> bool {
    loop_rows::owes(row, &events(store), TURN, 0)
}

/// `await-tools` 此刻是否还欠着这笔 `sent`。
fn awaits(store: &EventStore) -> bool {
    owes("await-tools", store)
}

/// 只落了 `request` 的一笔调用：还没判、还没派（授权行的前件）。
fn requested(store: &EventStore, id: &str) -> ToolPortCall {
    let call = port::Call::new("tool", id, 1);
    port::announce(store, &call, TURN, USER_SEQ).expect("request 落格");
    ToolPortCall {
        model_id: format!("call-{id}"),
        call,
        name: "shell".into(),
        // `risk = None` ⇒ 注册表里没有可判的能力（route-only），授权按 deferred 放行。
        risk: None,
        approved: false,
        invalid: false,
        granted: false,
        deny_why: None,
        sent: false,
        wake: None,
    }
}

/// 已开账、已放行、已派发的一笔调用：`request` + `sent` 都经引擎原语落过。
///
/// 两个旗标在这里直接补齐——本函数服务**关账**用例，「派出后欠一次应答」是前提
/// 不是被测点；派发这一步真的换债由场景一那条用例对。
fn opened(store: &EventStore, id: &str) -> ToolPortCall {
    let mut call = requested(store, id);
    port::begin(
        store,
        loop_rows::PORTS,
        port::Dispatch {
            call: &call.call,
            turn: TURN,
            produced_by: USER_SEQ,
            owed: true,
        },
    )
    .expect("sent 落格");
    call.granted = true;
    call.sent = true;
    call
}

fn outcome(model_id: &str, failure_kind: &str) -> DispatchOutcome {
    DispatchOutcome {
        call_id: model_id.into(),
        name: "shell".into(),
        text: "占位".into(),
        ok: false,
        failure_kind: failure_kind.into(),
        ..Default::default()
    }
}

fn tool_port(store: &Arc<EventStore>) -> ToolPort {
    let facts: Arc<dyn Store<Event = Event>> = store.clone();
    ToolPort::new(facts, TURN, USER_SEQ, 0)
}

/// **中止**：占位结果不是应答 ⇒ `failed{why:"aborted"}`，且这条调用**没有**
/// `response`；`await-tools` 由 `failed` 出口收清（判据 6 场景三的事实面）。
#[test]
fn an_aborted_call_closes_as_failed_not_as_an_answer() {
    let store = Arc::new(EventStore::new());
    let call = opened(&store, "tool-1-d1-b0-c0");
    assert!(awaits(&store), "落了 sent 就该欠一次应答");

    tool_port(&store).close_batch(
        std::slice::from_ref(&call),
        &[outcome(&call.model_id, ABORTED)],
    );

    assert_eq!(
        payload_of(&store, "failed"),
        json!({"port": "tool", "id": "tool-1-d1-b0-c0", "why": ABORTED}),
        "中止的成因要写在协议事实上，事后不必靠猜"
    );
    assert!(
        !kinds(&store).contains(&"response".to_string()),
        "占位结果不是工具回的，不能落应答"
    );
    assert!(!awaits(&store), "failed 出口必须收清这一笔");
}

/// **同批被跳过**：同样是没跑到，落 `failed{why:"not_executed"}`——与中止共用
/// 结果节点上那份词表（`ABORTED` / `NOT_EXECUTED`），节点侧与端口侧不会各说一套。
#[test]
fn a_skipped_call_closes_as_not_executed() {
    let store = Arc::new(EventStore::new());
    let call = opened(&store, "tool-1-d1-b0-c1");

    tool_port(&store).close_batch(
        std::slice::from_ref(&call),
        &[outcome(&call.model_id, NOT_EXECUTED)],
    );

    assert_eq!(payload_of(&store, "failed")["why"], json!(NOT_EXECUTED));
    assert!(!kinds(&store).contains(&"response".to_string()));
    assert!(!awaits(&store));
}

/// 连结果节点都没有（执行层整批没回来）⇒ 同样按「没跑到」收口，不留悬着的 `sent`。
#[test]
fn a_missing_outcome_closes_as_not_executed() {
    let store = Arc::new(EventStore::new());
    let call = opened(&store, "tool-1-d1-b0-c2");

    tool_port(&store).close_batch(std::slice::from_ref(&call), &[]);

    assert_eq!(payload_of(&store, "failed")["why"], json!(NOT_EXECUTED));
    assert!(!awaits(&store), "没有结果也不能让这一笔永远欠着");
}

/// 反向判据一：工具**跑完而失败**是应答。`success=false` 说的是内容，不是「没执行」
/// ——把它落成 `failed` 会让日志说「这一笔没跑到」，而它执行过。
#[test]
fn a_tool_that_ran_and_errored_is_still_an_answer() {
    let store = Arc::new(EventStore::new());
    let call = opened(&store, "tool-1-d1-b0-c3");

    tool_port(&store).close_batch(
        std::slice::from_ref(&call),
        &[outcome(&call.model_id, "error")],
    );

    assert_eq!(
        payload_of(&store, "response")["id"],
        json!("tool-1-d1-b0-c3"),
        "错误也是工具自己回的话"
    );
    assert!(
        !kinds(&store).contains(&"failed".to_string()),
        "跑完了就不能记成没跑到"
    );
    assert!(!awaits(&store));
}

/// 反向判据二：交回**审批卡**（`needs_approval`）也是应答——这一笔确实执行到了
/// 闸门并由工具自决收口，等用户是「下一次开账」的事（D-121 的审批作用），不是
/// 「这一笔没跑到」。
#[test]
fn a_call_that_returned_an_approval_card_is_an_answer() {
    let store = Arc::new(EventStore::new());
    let call = opened(&store, "tool-1-d1-b0-c4");

    tool_port(&store).close_batch(
        std::slice::from_ref(&call),
        &[outcome(&call.model_id, "needs_approval")],
    );

    assert!(kinds(&store).contains(&"response".to_string()));
    assert!(!kinds(&store).contains(&"failed".to_string()));
}

/// 边界：没放行的那几笔已由 `authz.denied` 关掉，关账不替它们补任何协议事实
/// ——补了就成了「一笔调用两条终态」，行表的 `until` 会同时看见 response 与 failed。
#[test]
fn a_denied_or_unsent_call_lands_nothing() {
    let store = Arc::new(EventStore::new());
    let mut denied = opened(&store, "tool-1-d1-b0-c5");
    denied.granted = false;
    let mut unsent = opened(&store, "tool-1-d1-b0-c6");
    unsent.sent = false;

    tool_port(&store).close_batch(
        &[denied.clone(), unsent.clone()],
        &[
            outcome(&denied.model_id, ABORTED),
            outcome(&unsent.model_id, NOT_EXECUTED),
        ],
    );

    let kinds = kinds(&store);
    assert!(
        !kinds.contains(&"failed".to_string()) && !kinds.contains(&"response".to_string()),
        "端口账已关的调用不再补记：{kinds:?}"
    );
}

/// 授权动作的策略：会话没设 `RISK_LEVEL` ⇒ 阈值走默认（与 `ToolPortPolicy::read`
/// 缺席时同一条），能力表为空（route-only 工具按 deferred 放行）。
fn policy() -> ToolPortPolicy {
    ToolPortPolicy {
        known: Vec::new(),
        threshold: CapabilityRiskLevel::default(),
        mode: String::new(),
    }
}

/// **场景一（审批唤醒）**：`authz.granted(port=tool)` 一落，`dispatch-tools` 就
/// 重新欠着；`begin_calls` 落 `sent` 后债位换到 `await-tools`。
///
/// 对的是**写方**（`ToolPort::authorize_calls` / `begin_calls`）而不是行声明：
/// 行层那几条合成用例（`loop_rows.test.rs`）只能证明「表认得 granted」，证明不了
/// 生产真的写出带 `port` / `id` 的载荷——键对不上时行永不开账，而日志一片绿。
#[test]
fn an_authorization_reopens_the_dispatch_debt_and_a_send_moves_to_the_await() {
    let store = Arc::new(EventStore::new());
    let port = tool_port(&store);
    let mut calls = [requested(&store, "tool-resume-1-tc-1-1")];

    assert!(owes("authorize-tool", &store), "请求落了、还没判");
    assert!(
        !owes("dispatch-tools", &store),
        "还没判 ⇒ 派发行不成立（fail-closed）"
    );

    port.authorize_calls(&policy(), &mut calls);

    assert!(calls[0].granted, "deferred 的调用交给下游路由闸门");
    assert_eq!(
        payload_of(&store, EVENT_AUTHZ_GRANTED)["id"],
        json!("tool-resume-1-tc-1-1"),
        "授权事实按 `(port, id)` 绑定：`id` 缺一格，派发行就永远不认它"
    );
    assert!(!owes("authorize-tool", &store), "判过了 ⇒ 授权行关账");
    assert!(
        owes("dispatch-tools", &store),
        "审批落 granted ⇒ 派发重新成为一笔待还的账（判据 6 场景一）"
    );

    port.begin_calls(&mut calls);

    assert!(calls[0].sent, "派发行欠 ⇒ `sent` 落格并回报旗标");
    assert!(!owes("dispatch-tools", &store), "派出 ⇒ 派发行关账");
    assert!(awaits(&store), "换一笔债在等应答");
}

// ── 唤醒幕（ 步 5d）：用户动作落成的那一笔 `request` ──────────────────
//
// 唤醒路径的对称面在别处：唤醒幕没有模型应答，工具批的入口是**用户动作**。
// 用户动作先落成一笔 `request(port=tool)`，之后与批次走**同一条**授权 / 派发链。
// 这一组用例钉的是写方与读侧：那一笔的载荷形状（读侧靠它还原）、授权行认不认它、
// 关账之后还在不在。

/// 用户动作落成的那一笔：`request(port=tool)`，**只有**这一笔。
///
/// 关键在「只有」：授权与派发由行表的 `authorize-tool` / `dispatch-tools` 两行
/// 承担，写方在这里一并落掉就等于恢复路径又长出一条绕过行表的旁路。
#[test]
fn a_wake_request_lands_as_a_port_request_carrying_the_resume_marker() {
    let store = Arc::new(EventStore::new());
    let port = tool_port(&store);

    port.land_wake(&WakeSpec {
        kind: "approve",
        target: "tc-1",
        tool: "vdfs_read",
        args: &json!({ "path": "a.md", "approved": true }),
        wire_id: "w-1",
        turn_root: "root-1",
        intent: &WakeIntent::Execute,
    })
    .expect("唤醒请求落格");

    let facts = events(&store);
    let req = facts
        .iter()
        .find(|e| e.kind == EVENT_REQUEST)
        .expect("请求格在");
    assert_eq!(req.payload["port"], json!("tool"));
    assert_eq!(
        req.payload["id"],
        json!("tool-resume-1-tc-1-1"),
        "唤醒账的 id 带轮号 / 目标 / 尝试号（重启后仍可对上）"
    );
    assert_eq!(req.payload["attempt"], json!(1));
    assert_eq!(req.payload["resume"], json!("approve"), "唤醒标记在载荷上");
    assert_eq!(req.payload["target"], json!("tc-1"), "目标节点 id 在载荷上");
    assert_eq!(req.payload["tool"], json!("vdfs_read"));
    assert_eq!(req.payload["wire_id"], json!("w-1"));

    assert!(
        !facts.iter().any(|e| e.kind == EVENT_AUTHZ_GRANTED),
        "用户动作落格时**不**落授权：授权是行表的动作"
    );
    assert!(
        !facts.iter().any(|e| e.kind == EVENT_SENT),
        "用户动作落格时**不**落 `sent`：派发是行表的动作"
    );
    assert!(
        owes("authorize-tool", &store),
        "唤醒请求一落，授权行就欠着（它自己会认领这一笔）"
    );
}

/// 读侧：唤醒那一笔能被还原成派发 / 授权动作要的那份调用。
///
/// 还原出来的东西缺一格，引擎就会拿空的目标去跑工具（结果挂不回原来那张卡）。
#[test]
fn the_wake_read_side_restores_the_intent_and_the_target() {
    let store = Arc::new(EventStore::new());
    let port = tool_port(&store);
    port.land_wake(&WakeSpec {
        kind: "approve",
        target: "tc-1",
        tool: "vdfs_read",
        args: &json!({ "path": "a.md", "approved": true }),
        wire_id: "w-1",
        turn_root: "root-1",
        intent: &WakeIntent::Execute,
    })
    .expect("唤醒请求落格");

    let batch = port.wake_batch().expect("唤醒请求读得回");
    assert_eq!(batch.calls.len(), 1, "唤醒只还原被唤醒的那一笔");
    let call = &batch.calls[0];
    assert_eq!(call.model_id, "tc-1", "目标节点 id 落在模型侧 id 上");
    assert_eq!(call.name, "vdfs_read");
    assert_eq!(call.call.id, "tool-resume-1-tc-1-1");
    assert!(call.approved, "参数里带 `approved: true` ⇒ 已由人工确认过");
    assert!(
        matches!(call.wake, Some(WakeIntent::Execute)),
        "唤醒意图从载荷还原：{:?}",
        call.wake
    );
    assert!(!call.granted, "还没判 ⇒ 未放行");

    let tc = &batch.turn.tool_calls[0];
    assert_eq!(tc.id.as_deref(), Some("tc-1"), "结果挂回原来那张卡");
    assert_eq!(tc.name.as_deref(), Some("vdfs_read"));
    assert_eq!(tc.wire_id.as_deref(), Some("w-1"));
    assert_eq!(tc.arguments["path"], json!("a.md"));
}

/// 用户**拒绝**：意图从载荷还原成「有结果的拒绝」，授权行落 `authz.denied`。
///
/// 反向的那一半同样重要：拒绝之后派发尾要落 `failed{user_rejected}`，
/// 否则创建行的集合债永远差一个终局（见 `deny-tools` 行）。
#[test]
fn a_wake_that_the_user_rejected_reads_back_as_a_denial() {
    let store = Arc::new(EventStore::new());
    let port = tool_port(&store);
    let intent = WakeIntent::Result {
        text: "用户拒绝执行: 有风险".into(),
        ok: false,
        denied: true,
    };
    port.land_wake(&WakeSpec {
        kind: "reject",
        target: "tc-1",
        tool: "shell",
        args: &json!({ "cmd": "rm -rf /" }),
        wire_id: "w-1",
        turn_root: "root-1",
        intent: &intent,
    })
    .expect("唤醒请求落格");

    let mut batch = port.wake_batch().expect("唤醒请求读得回");
    match &batch.calls[0].wake {
        Some(WakeIntent::Result { text, ok, denied }) => {
            assert_eq!(text, "用户拒绝执行: 有风险");
            assert!(!ok);
            assert!(denied);
        }
        other => panic!("唤醒意图还原错了：{other:?}"),
    }

    port.authorize_wake(&mut batch.calls);
    assert!(!batch.calls[0].granted, "拒绝不放行：执行层不会跑它");
    assert_eq!(batch.calls[0].deny_why, Some("user_rejected"));
    assert!(
        events(&store).iter().any(|e| e.kind == EVENT_AUTHZ_DENIED),
        "拒绝落 `authz.denied`（授权行由此关账）"
    );
}

/// 关账之后读不回来了：这一笔的派发已经发生过，动作不该再跑第二遍。
#[test]
fn a_closed_wake_request_is_not_read_back() {
    let store = Arc::new(EventStore::new());
    let port = tool_port(&store);
    port.land_wake(&WakeSpec {
        kind: "approve",
        target: "tc-1",
        tool: "vdfs_read",
        args: &json!({ "path": "a.md" }),
        wire_id: "w-1",
        turn_root: "root-1",
        intent: &WakeIntent::Execute,
    })
    .expect("唤醒请求落格");

    let mut batch = port.wake_batch().expect("唤醒请求读得回");
    port.authorize_wake(&mut batch.calls);
    assert!(batch.calls[0].granted, "批准 ⇒ 放行");
    port.begin_calls(&mut batch.calls);
    assert!(batch.calls[0].sent, "派发行欠 ⇒ 落 `sent`");
    port.close(&batch.calls[0], true);

    assert!(
        port.wake_batch().is_none(),
        "已关账的唤醒请求不再读回（否则同一笔会派发两遍）"
    );
}
