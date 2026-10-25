//! `stage_exec` 的单元测试 —— 外环那两个环步各自落的格。
//!
//! ## 这些用例钉的是哪四件事
//!
//! 1. **宣布落的是一条协议请求**，键含幕锚与请求轮次（不是「随手一条 request」）；
//! 2. **同一轮的两幕不撞号**：续写幕复用轮号、`request_round` 从 0 重来，逻辑 id
//!    只带轮号与往返号会撞成同一个 `id`，而首幕已经有过 `sent` ⇒ 派发行不欠、
//!    整轮在派发处被拒（D-168 裁决 4 的现成后果）；
//! 3. **没有交接件的派出判死那笔遗留请求**，不是静默地什么也不做（D-168 的
//!    `port::kill_dangling` 在 `turn` 端口上的那一支）；
//! 4. **无事可还就停**：不欠账的一幕驱动一次不多写一格。
//!
//! ## 为什么这里不跑一整个幕
//!
//! 派出那一半的执行体是一整幕（`execute_turn`），它的判据在 `turn_exec.test.rs`。
//! 这里要问的是**环步的账**（宣布 / 判死 / 停机），跑真模型会把两组判据混成一堆
//! 互相解释不了的断言。假 provider 因此只会被问「有没有人调我」——它一被调用就
//! 返回错，用例当场红。

use std::sync::Arc;

use async_trait::async_trait;

use super::super::chat_loop::drive::begin_drive;
use super::super::chat_loop::inputs::TurnInputs;
use super::super::chat_loop::state::{SessionContext, TurnRequest, TurnState};
use super::super::loop_rows::PORTS;
use super::super::port::{self, Call, Dispatch, DispatchError};
use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};
use super::super::turn_exec::{ExecTurnResult, TurnExecExit};
use super::{drive_stage, run_announce_turn, run_dispatch_turn, StageStep};
use crate::symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use crate::symbio_core::principal::{Principal, PrincipalKind};
use crate::symbio_core::{
    CapabilityMeta, Event, EventStore, ExecEventSink, ExecTranscriptWriter, ModelProvider,
    PluginError, PluginInvokeRequest, PluginSimpleRequest, Seq, Store, TurnOutput, EVENT_FAILED,
    EVENT_REQUEST, EVENT_SENT,
};

// ── 夹具 ────────────────────────────────────────────────────────────────

/// 帧口：外环不产帧（它不调模型、不上线），所以这里只需要一个能塞进 sink 的空口。
struct NoopSink;

#[async_trait]
impl ExecTranscriptWriter for NoopSink {
    async fn apply(&self, _m: ChatNode) {}
}

/// 假 provider：**只会被问「有没有人调我」**。
///
/// 一被调用就返回错——这样「宣布那一行偷偷跑了整幕」会是一条红，而不是一条
/// 看起来正常的通过。
struct NoModel;

#[async_trait]
impl ModelProvider for NoModel {
    fn provider_id(&self) -> &str {
        "stage-exec-mock"
    }
    fn api_protocol(&self) -> &str {
        "openai_chat"
    }
    fn rate_limit_ms(&self) -> u64 {
        0
    }
    fn max_context_tokens(&self) -> u32 {
        8192
    }
    async fn effective_context_tokens(&self) -> u32 {
        8192
    }
    async fn execute_turn(
        &self,
        _system: &str,
        _messages: &[ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &crate::symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError(
            "外环的这两个环步不该调模型".into(),
        ))
    }
}

/// 一条用户消息节点（`begin_drive` 从它取本轮发言）。
fn user_node(id: &str, text: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        principal: Some(Principal::new(PrincipalKind::Person, "user")),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 拆开的夹具：字段各自成局部变量，`StageStep` 才能同时借到 `&mut context`
/// 与 `&orchestrator`（同一个结构体的字段分两次借，一次可变一次不可变是不允许的）。
struct Fixture {
    facts: Arc<EventStore>,
    sink: ExecEventSink,
    orchestrator: Arc<super::super::chat_loop::ChatOrchestrator>,
    context: SessionContext,
    inputs: TurnInputs,
}

impl Fixture {
    fn new(messages: Vec<ChatNode>) -> Self {
        let facts = Arc::new(EventStore::new());
        let port = Arc::new(FakeSessionPort::new(messages.clone()).with_facts(facts.clone()));
        let spec = turn_spec(
            "s-stage",
            std::env::temp_dir(),
            std::env::temp_dir(),
            default_settings(),
        );
        let session = Arc::new(loop_session(spec, facts.clone(), port));
        let sink = ExecEventSink::direct(Arc::new(NoopSink));
        let orchestrator = Arc::new(super::super::chat_loop::ChatOrchestrator {
            provider: Arc::new(NoModel),
            parent: None,
            context_limit: 8192,
            session: session.clone(),
            writer: Arc::new(NoopSink),
            compose_enabled: false,
            opening_enabled: false,
            presentation_tree: crate::symbio_core::node_build::PresentationTree::Grouped,
        });
        Self {
            facts,
            sink,
            orchestrator,
            context: SessionContext {
                messages,
                session,
                consolidated: None,
            },
            inputs: TurnInputs {
                root_id: "r-1".into(),
                system_prompt: "system".into(),
                tools: Vec::new(),
                request_sections: Vec::new(),
            },
        }
    }
}

fn request() -> TurnRequest {
    TurnRequest {
        auto_compress: false,
        load_history: true,
        system_prompt: None,
    }
}

/// 一轮的**全部** `turn` 端口请求格（按落格顺序）。
fn turn_requests(facts: &[Event]) -> Vec<&Event> {
    facts
        .iter()
        .filter(|e| {
            e.kind == EVENT_REQUEST
                && e.payload.get("port").and_then(|v| v.as_str()) == Some("turn")
        })
        .collect()
}

// ── 宣布：落的是协议请求，键含幕锚 ───────────────────────────────────────

/// 宣布落的是一条 `request(port=turn)`，逻辑 id 含**幕锚**与请求轮次，交接件同时到位。
#[tokio::test]
async fn announce_lands_a_turn_request_keyed_by_the_stage_anchor() {
    let Fixture {
        facts,
        sink,
        orchestrator,
        mut context,
        inputs,
    } = Fixture::new(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let store: Arc<dyn Store<Event = Event>> = facts.clone();
    let mut call = None;
    let mut outcome: Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>> = None;

    {
        let mut step = StageStep {
            orchestrator: &orchestrator,
            orchestrator_arc: &orchestrator,
            ctx: &ctx,
            store: &store,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            inputs: &inputs,
            user_text: "你好",
            consolidation: None,
            call: &mut call,
            outcome: &mut outcome,
        };
        run_announce_turn(&mut step).await.expect("宣布落得了格");
    }

    let all = facts.range(Seq::new(0));
    let requests = turn_requests(&all);
    assert_eq!(requests.len(), 1, "只宣布一笔：{all:?}");
    let req = requests[0];
    assert_eq!(
        req.event_id,
        format!("request-turn-turn-0-d{}-r0-1", turn.drive_seq),
        "协议格的键是 `(port, id, attempt)`"
    );
    assert_eq!(req.payload["attempt"], 1);
    assert_eq!(req.turn, turn.turn_no);
    assert_eq!(
        req.produced_by,
        Some(turn.user_seq),
        "协议事实是断言类：溯源必须指向本轮用户格"
    );
    assert!(req.payload.get("via").is_none(), "这是本幕自己的往返");

    let handed = call.expect("宣布的产物是那两个环步之间的交接件");
    assert_eq!(handed.port, "turn");
    assert_eq!(handed.id, format!("turn-0-d{}-r0", turn.drive_seq));
    assert!(outcome.is_none(), "宣布不产出幕的结局");
}

/// 同一轮的**两幕**不撞号：幕锚不同 ⇒ 逻辑 id 不同 ⇒ 两条 `request` 都在。
///
/// 这条用例是「为什么 id 里必须有幕锚」的反面证据：去掉 `d{drive_seq}` 之后
/// 第二次宣布会落成 `Duplicate`（被当作「已记录」静默吞掉），而首幕那笔早已
/// 有 `sent` —— 派发行不欠，本幕一条请求也没有。
#[tokio::test]
async fn two_stages_of_one_turn_do_not_collide_on_the_logical_id() {
    let Fixture {
        facts,
        sink,
        orchestrator,
        mut context,
        inputs,
    } = Fixture::new(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let store: Arc<dyn Store<Event = Event>> = facts.clone();
    let mut call = None;
    let mut outcome: Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>> = None;

    // 首幕：宣布。
    {
        let mut step = StageStep {
            orchestrator: &orchestrator,
            orchestrator_arc: &orchestrator,
            ctx: &ctx,
            store: &store,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            inputs: &inputs,
            user_text: "你好",
            consolidation: None,
            call: &mut call,
            outcome: &mut outcome,
        };
        run_announce_turn(&mut step)
            .await
            .expect("首幕宣布落得了格");
    }
    let first = call.take().expect("首幕的交接件");

    // 续写幕：轮号不变、请求轮次从 0 重来、幕锚前进。
    turn.request_round = 0;
    turn.drive_seq += 100;
    {
        let mut step = StageStep {
            orchestrator: &orchestrator,
            orchestrator_arc: &orchestrator,
            ctx: &ctx,
            store: &store,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            inputs: &inputs,
            user_text: "你好",
            consolidation: None,
            call: &mut call,
            outcome: &mut outcome,
        };
        run_announce_turn(&mut step)
            .await
            .expect("续写幕宣布落得了格");
    }
    let second = call.take().expect("续写幕的交接件");

    assert_ne!(first.id, second.id, "幕锚进了逻辑 id，两幕才是两笔账");
    let all = facts.range(Seq::new(0));
    let requests = turn_requests(&all);
    assert_eq!(requests.len(), 2, "两幕各留一条请求：{all:?}");
    assert_eq!(requests[0].event_id, format!("request-turn-{}-1", first.id));
    assert_eq!(
        requests[1].event_id,
        format!("request-turn-{}-1", second.id)
    );
}

// ── 派出：没有交接件时判死遗留请求 ───────────────────────────────────────

/// 没有交接件（上一段生命死在宣布与派出之间）⇒ 把最老的那笔判死，不是静默跳过。
#[tokio::test]
async fn a_dispatch_without_a_handover_kills_the_dangling_request() {
    let Fixture {
        facts,
        sink,
        orchestrator,
        mut context,
        inputs,
    } = Fixture::new(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let store: Arc<dyn Store<Event = Event>> = facts.clone();

    // 上一段生命的遗留：宣布过、没派出，等它的 future 已经不在了。
    let orphan = Call::new("turn", "turn-0-d0-r0", 1);
    port::announce(&*store, &orphan, turn.turn_no, turn.user_seq).expect("遗留请求入格");

    let mut call = None;
    let mut outcome: Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>> = None;
    {
        let mut step = StageStep {
            orchestrator: &orchestrator,
            orchestrator_arc: &orchestrator,
            ctx: &ctx,
            store: &store,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            inputs: &inputs,
            user_text: "你好",
            consolidation: None,
            call: &mut call,
            outcome: &mut outcome,
        };
        run_dispatch_turn(&mut step)
            .await
            .expect("判死不算执行体失败");
    }

    let all = facts.range(Seq::new(0));
    let failed: Vec<&Event> = all.iter().filter(|e| e.kind == EVENT_FAILED).collect();
    assert_eq!(failed.len(), 1, "遗留请求被判死一次：{all:?}");
    assert_eq!(failed[0].payload["id"], "turn-0-d0-r0");
    assert_eq!(failed[0].payload["why"], "no_pending_round");
    assert!(
        !all.iter().any(|e| e.kind == EVENT_SENT),
        "判死不是派出——一条 `sent` 都不该有"
    );
    assert!(outcome.is_none(), "没派出去就没有幕的结局");
}

/// 没有交接件、也没有遗留请求 ⇒ 一格不写（判死是**有对象才动**）。
#[tokio::test]
async fn a_dispatch_without_a_handover_and_nothing_dangling_writes_nothing() {
    let Fixture {
        facts,
        sink,
        orchestrator,
        mut context,
        inputs,
    } = Fixture::new(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let store: Arc<dyn Store<Event = Event>> = facts.clone();
    let before = facts.range(Seq::new(0)).len();

    let mut call = None;
    let mut outcome: Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>> = None;
    {
        let mut step = StageStep {
            orchestrator: &orchestrator,
            orchestrator_arc: &orchestrator,
            ctx: &ctx,
            store: &store,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            inputs: &inputs,
            user_text: "你好",
            consolidation: None,
            call: &mut call,
            outcome: &mut outcome,
        };
        run_dispatch_turn(&mut step)
            .await
            .expect("无事可做也是成功");
    }

    assert_eq!(
        facts.range(Seq::new(0)).len(),
        before,
        "没有对象可判死时不许造账"
    );
    assert!(outcome.is_none());
}

// ── 停机：本幕不欠账就一格不动 ───────────────────────────────────────────

/// 本幕的 `turn` 往返已走完（请求 / 派出 / 应答三格齐）⇒ 外环立刻收工。
///
/// 这条钉的是 `drive_stage` 的**停机判据**：选不出可还的债就返回，不拿假成功
/// 收场，也不顺手再宣布一笔。
#[tokio::test]
async fn drive_stage_stops_when_the_stage_owes_nothing() {
    let Fixture {
        facts,
        sink: _sink,
        orchestrator,
        mut context,
        inputs,
    } = Fixture::new(vec![user_node("m1", "你好")]);
    let mut turn = TurnState::default();
    begin_drive(&context, &mut turn, &request(), None).expect("新轮开得了");
    let store: Arc<dyn Store<Event = Event>> = facts.clone();

    // 手工把这一幕的往返走到终局：宣布 → 派出 → 应答。
    let call = Call::new("turn", "turn-0-done", 1);
    port::announce(&*store, &call, turn.turn_no, turn.user_seq).expect("请求入格");
    port::begin(
        &*store,
        PORTS,
        Dispatch {
            call: &call,
            turn: turn.turn_no,
            produced_by: turn.user_seq,
            row: "dispatch-turn",
            now: turn.now_ms,
            awaited: false,
        },
    )
    .expect("派出入格");
    port::succeed(&*store, &call, turn.turn_no, turn.user_seq).expect("应答入格");
    let before = facts.range(Seq::new(0)).len();

    let out = drive_stage(super::StageDrive {
        orchestrator: &orchestrator,
        orchestrator_arc: &orchestrator,
        ctx: &(Arc::new(PluginSimpleRequest::new(None, None)) as Arc<dyn PluginInvokeRequest>),
        sink: &_sink,
        context: &mut context,
        turn: &mut turn,
        inputs: &inputs,
        user_text: "你好",
        consolidation: None,
    })
    .await
    .expect("不欠账不是错误");

    assert!(out.is_none(), "本幕没有新结局");
    assert_eq!(
        facts.range(Seq::new(0)).len(),
        before,
        "不欠账的一幕一格都不该多"
    );
    let all = facts.range(Seq::new(0));
    assert_eq!(
        turn_requests(&all).len(),
        1,
        "停机不是「再宣布一笔」：请求仍只有手工那一笔"
    );
    // 这三个 kind 在手工那一段各落了一格——把「外环没重跑」写成名单而不是计数，
    // 少一格或多一格都能看出是哪里动的。
    let kinds: Vec<&str> = all
        .iter()
        .filter(|e| matches!(e.kind.as_str(), EVENT_REQUEST | EVENT_SENT | "response"))
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(kinds, vec![EVENT_REQUEST, EVENT_SENT, "response"]);
}
