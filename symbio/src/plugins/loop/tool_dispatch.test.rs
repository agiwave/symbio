//! 工具端口三笔债的判据（T-244 步 4）：授权 / 派发 / 关账由**生产代码**写，
//! 行表按它们判欠不欠。
//!
//! ## 场景三（中止唤醒）：占位结果不是应答
//!
//! 一条 `sent` 之后只有两种收场：工具回了话（`response`），或执行层没跑到它
//! （`failed{why}`）。判据是「日志说的与发生的事一致」——批尾收口给没跑到的调用
//! 补的那条占位结果是**框架的交代**（给模型一句话、给前端一张卡），不是工具的
//! 应答；把它算成应答，中止的调用就在日志里写成「回来了」，`await-tools` 的
//! `failed` 出口永远等不到人（PL-017 §5 判据 6 的中止场景就是这么空的）。
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
use super::{DispatchOutcome, ToolPort, ToolPortCall, ToolPortPolicy};
use crate::symbio_core::{CapabilityRiskLevel, Event, EventStore, Seq, Store, EVENT_AUTHZ_GRANTED};

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
        granted: false,
        sent: false,
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
/// 闸门并由工具自决收口，等用户是「下一次开账」的事（ADR-121 的审批作用），不是
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
