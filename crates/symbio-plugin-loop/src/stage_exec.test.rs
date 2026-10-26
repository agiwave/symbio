//! `stage_exec` 的单元测试 —— 外环那三个环步各自落的格。
//!
//! ## 这些用例钉的是哪几件事
//!
//! 1. **准备落的是一格阶段事实**（`turn.stage{stage:"inputs"}`），并把输入交给下一个
//!    环步——「做完了」不再只是函数返回值；
//! 2. **定稿答话轮在准备段收束**：那一幕落 `result:"closed"`、不交输入（否则环会为
//!    一个已经答完的轮再派一次往返）；
//! 3. **宣布落的是一条协议请求**，键含幕锚与请求轮次（不是「随手一条 request」）；
//! 4. **同一轮的两幕不撞号**：续写幕复用轮号、`request_round` 从 0 重来，逻辑 id
//!    只带轮号与往返号会撞成同一个 `id`，而首幕已经有过 `sent` ⇒ 派发行不欠、
//!    整轮在派发处被拒（D-168 裁决 4 的现成后果）；
//! 5. **没有交接件的派出判死那笔遗留请求**，不是静默地什么也不做；
//! 6. **无事可还就停**：不欠账的一幕驱动一次不多写一格；
//! 7. **重试的新支在准备段登记**：一条 `turn.branch`（`into` 用这一幕才存在的新根 id），
//!    且 `retry_of` 取走即消费——续写幕不会再开一支。
//!
//! ## 为什么这里不跑一整个幕
//!
//! 派出那一半的执行体是一整幕（`execute_turn`），它的判据在 `turn_exec.test.rs`。
//! 这里要问的是**环步的账**（准备 / 宣布 / 判死 / 停机），跑真模型会把两组判据混成
//! 一堆互相解释不了的断言。假 provider 因此只会被问「有没有人调我」——它一被调用
//! 就返回错，用例当场红。

use std::sync::Arc;

use async_trait::async_trait;

use super::super::chat_loop::drive::begin_drive;
use super::super::chat_loop::inputs::TurnInputs;
use super::super::chat_loop::state::{SessionContext, TurnExit, TurnRequest, TurnState};
use super::super::chat_loop::AnsweredTurn;
use super::super::loop_rows::PORTS;
use super::super::port::{self, Call, Dispatch};
use super::super::stage_events::{land, StageFact, StageName};
use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};
use super::{
    drive_stage, run_announce_turn, run_dispatch_turn, run_prepare_turn, StageDrive, StageStep,
};
use symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use symbio_core::principal::{Principal, PrincipalKind};
use symbio_core::{
    CapabilityMeta, Event, EventStore, ExecEventSink, ExecTranscriptWriter, ModelProvider,
    PluginError, PluginInvokeRequest, PluginSimpleRequest, Seq, Store, TurnOutput, EVENT_FAILED,
    EVENT_REQUEST, EVENT_SENT,
};
use symbio_core::{Entity, Verb, EVENT_ASSISTANT_FINAL, EVENT_TURN_BRANCH};

// ── 夹具 ────────────────────────────────────────────────────────────────

/// 帧口：外环不上线，所以只需要一个能塞进 sink 的空口。
struct NoopSink;

#[async_trait]
impl ExecTranscriptWriter for NoopSink {
    async fn apply(&self, _m: ChatNode) {}
}

/// 假 provider：**只会被问「有没有人调我」**。
///
/// 一被调用就返回错——这样「准备 / 宣布那一行偷偷跑了整幕」会是一条红，而不是一条
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
        _env: &symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError(
            "外环的这几个环步不该调模型".into(),
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

/// 拆开的夹具：`StageStep` 要同时借到 `&mut context` 与 `&orchestrator`，
/// 而同一个结构体的字段分两次借（一次可变一次不可变）是不允许的，故先拆开。
struct Harness {
    facts: Arc<EventStore>,
    store: Arc<dyn Store<Event = Event>>,
    sink: ExecEventSink,
    ctx: Arc<dyn PluginInvokeRequest>,
    orchestrator: Arc<super::super::chat_loop::ChatOrchestrator>,
    context: SessionContext,
    req: TurnRequest,
    turn: TurnState,
}

impl Harness {
    /// 判决与措辞**都关掉**：这些用例问的是准备 / 宣布 / 派出三笔账本身，不是判决
    /// 判成什么。关掉之后 `policy.classify` / `policy.compress` 都不开账，
    /// 准备段只剩「收集 + 召回 + 落出口」。
    fn new(messages: Vec<ChatNode>) -> Self {
        let facts = Arc::new(EventStore::new());
        let port = Arc::new(FakeSessionPort::new(messages.clone()).with_facts(facts.clone()));
        let mut settings = default_settings();
        settings.classify_enabled = false;
        settings.compose_enabled = false;
        settings.auto_compress = false;
        let spec = turn_spec(
            "s-stage",
            std::env::temp_dir(),
            std::env::temp_dir(),
            settings,
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
            presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
        });
        let store: Arc<dyn Store<Event = Event>> = facts.clone();
        Self {
            facts,
            store,
            sink,
            ctx: Arc::new(PluginSimpleRequest::new(None, None)),
            orchestrator,
            context: SessionContext {
                messages,
                session,
                consolidated: None,
            },
            req: TurnRequest {
                auto_compress: false,
                load_history: true,
                system_prompt: None,
            },
            turn: TurnState::default(),
        }
    }

    /// 开一幕（用户格 + 配置 + 轮次事实先落，后续格才有幕锚可用）。
    fn open(&mut self) {
        begin_drive(&self.context, &mut self.turn, &self.req, None).expect("新轮开得了");
    }

    /// 组一台 host：三个交接件（输入 / 待派请求 / 中断）由调用方持有。
    fn step<'a>(
        &'a mut self,
        ready: &'a mut Option<TurnInputs>,
        call: &'a mut Option<Call>,
        interrupted: &'a mut Option<TurnExit>,
    ) -> StageStep<'a> {
        StageStep {
            orchestrator: &self.orchestrator,
            orchestrator_arc: &self.orchestrator,
            ctx: &self.ctx,
            store: &self.store,
            sink: &self.sink,
            context: &mut self.context,
            turn: &mut self.turn,
            req: &self.req,
            user_text: "你好",
            inputs: ready,
            call,
            interrupted,
        }
    }
}

/// 一轮里全部的 `turn` 端口请求格（按落格顺序）。
fn turn_requests(facts: &[Event]) -> Vec<&Event> {
    facts
        .iter()
        .filter(|e| {
            e.kind == EVENT_REQUEST
                && e.payload.get("port").and_then(|v| v.as_str()) == Some("turn")
        })
        .collect()
}

/// 本幕的 `inputs` 阶段格。
fn inputs_fact(facts: &[Event]) -> Option<&Event> {
    facts.iter().find(|e| {
        e.kind == "turn.stage" && e.payload.get("stage").and_then(|v| v.as_str()) == Some("inputs")
    })
}

/// 本幕落下的分支格（`turn.branch`）：一次重试一条。
fn branch_facts(h: &Harness) -> Vec<Event> {
    h.facts
        .range(Seq::new(0))
        .into_iter()
        .filter(|e| e.kind == EVENT_TURN_BRANCH)
        .collect()
}

// ── 准备：落出口、交输入 ─────────────────────────────────────────────────

/// 准备落的是一格 `inputs` 阶段事实，输入经交接件交给下一个环步。
#[tokio::test]
async fn prepare_lands_the_inputs_stage_and_hands_the_inputs_over() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;

    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_prepare_turn(&mut step).await.expect("准备落得了格");
    }

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("准备段的出口进了日志");
    assert_eq!(landed.payload["result"], "ready");
    assert_eq!(landed.payload["round"], 0);
    assert_eq!(landed.turn, h.turn.turn_no);

    let inputs = ready.expect("输入是这一环步的交接件");
    assert!(!inputs.root_id.is_empty(), "根骨架已分配");
    assert!(call.is_none(), "准备不宣布任何请求");
    assert!(interrupted.is_none(), "准备不产出幕的结局");
}

/// 重试开出的**新支在准备段登记**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 钉三件事，缺一件整套分支就读不对：
/// - `into` 用的是**这一幕才存在的新根 id**（`retry_of` 只是寄存的旧根，新根要到输入
///   备好才有；早一格落它就只能按轮号归组，而一幕一个根、幕间续写会让同轮开出多个根）；
/// - 它是断言类事实 ⇒ 溯源指向本轮用户格（I2，缺了它整条 WAL 重放会被判违规）；
/// - `take()` 即消费 ⇒ **第二幕不再另开一支**（否则幕间续写会凭空多出一个候选，
///   用户看到 `2/3` 却只重试过一次）。
#[tokio::test]
async fn prepare_lands_one_branch_fact_for_a_retry_and_consumes_the_registration() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    h.turn.retry_of = Some("t-old".to_string());

    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_prepare_turn(&mut step).await.expect("准备落得了格");
    }
    let new_root = ready.take().expect("输入是这一环步的交接件").root_id;
    assert!(
        h.turn.retry_of.is_none(),
        "寄存的旧根被取走：同一幕不会落两条分支格"
    );

    let landed = branch_facts(&h);
    assert_eq!(landed.len(), 1, "一次重试只开一支：{landed:?}");
    let branch = &landed[0];
    assert_eq!(branch.payload["of"], "t-old");
    assert_eq!(branch.payload["into"], new_root.as_str());
    assert_eq!(
        branch.produced_by,
        Some(h.turn.user_seq),
        "分支格是断言类：溯源必须指向本轮用户格"
    );
    assert!(!branch.actor.is_empty(), "actor 取本轮主体，不留空");

    // 续写幕：幕锚前进，`retry_of` 已空 ⇒ 事实面上一条分支格都不该再多。
    h.turn.request_round = 0;
    h.turn.drive_seq += 100;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_prepare_turn(&mut step)
            .await
            .expect("续写幕准备落得了格");
    }
    assert_eq!(branch_facts(&h).len(), 1, "幕间续写不再开第二支");
}

/// 定稿答话轮：准备段就地收束 ⇒ 落 `result:"closed"`、**不**交输入。
///
/// 这一格是「环别再派一次」的唯一依据。只落 `ready` 的话，收束的那一幕在事实面上
/// 与「还没准备」同形：环会重来一遍，而第二遍已经没有寄存的那体答话，于是它照常
/// 返回输入、照常派出一次往返——一个已经答完的轮会再答一次。
#[tokio::test]
async fn prepare_lands_closed_when_the_turn_answers_in_place() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    // 判决执行体寄存下来的那体答话（`run_classify` 的产出形态）。
    h.turn.head_answer = Some(AnsweredTurn {
        utterance: "你好".into(),
        reply: "你好，有什么可以帮你？".into(),
        cost_ms: 0,
        dialog: super::super::turn_runner::DialogFacts {
            id: "a-1".into(),
            reason: "answered".into(),
            exclude_from_context: false,
        },
        reasoning: None,
    });
    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;

    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_prepare_turn(&mut step).await.expect("收束不是失败");
    }

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("收束也要落格（否则与「还没准备」同形）");
    assert_eq!(landed.payload["result"], "closed");
    assert!(ready.is_none(), "收束的一幕没有输入");
    assert!(
        !all.iter().any(|e| e.kind == EVENT_REQUEST),
        "收束的一幕不宣布任何往返"
    );
}

// ── 宣布：落的是协议请求，键含幕锚 ───────────────────────────────────────

/// 宣布落的是一条 `request(port=turn)`，逻辑 id 含**幕锚**与请求轮次，交接件同时到位。
#[tokio::test]
async fn announce_lands_a_turn_request_keyed_by_the_stage_anchor() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;

    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_announce_turn(&mut step).await.expect("宣布落得了格");
    }

    let all = h.facts.range(Seq::new(0));
    let requests = turn_requests(&all);
    assert_eq!(requests.len(), 1, "只宣布一笔：{all:?}");
    let req = requests[0];
    assert_eq!(
        req.event_id,
        format!("request-turn-turn-0-d{}-r0-1", h.turn.drive_seq),
        "协议格的键是 `(port, id, attempt)`"
    );
    assert_eq!(req.payload["attempt"], 1);
    assert_eq!(req.turn, h.turn.turn_no);
    assert_eq!(
        req.produced_by,
        Some(h.turn.user_seq),
        "协议事实是断言类：溯源必须指向本轮用户格"
    );
    assert!(req.payload.get("via").is_none(), "这是本幕自己的往返");

    let handed = call.expect("宣布的产物是那两个环步之间的交接件");
    assert_eq!(handed.port, "turn");
    assert_eq!(handed.id, format!("turn-0-d{}-r0", h.turn.drive_seq));
    assert!(interrupted.is_none(), "宣布不产出幕的结局");
}

/// 同一轮的**两幕**不撞号：幕锚不同 ⇒ 逻辑 id 不同 ⇒ 两条 `request` 都在。
///
/// 这条用例是「为什么 id 里必须有幕锚」的反面证据：去掉 `d{drive_seq}` 之后
/// 第二次宣布会落成 `Duplicate`（被当作「已记录」静默吞掉），而首幕那笔早已
/// 有 `sent` —— 派发行不欠，本幕一条请求也没有。
#[tokio::test]
async fn two_stages_of_one_turn_do_not_collide_on_the_logical_id() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;

    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_announce_turn(&mut step)
            .await
            .expect("首幕宣布落得了格");
    }
    let first = call.take().expect("首幕的交接件");

    // 续写幕：轮号不变、请求轮次从 0 重来、幕锚前进。
    h.turn.request_round = 0;
    h.turn.drive_seq += 100;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_announce_turn(&mut step)
            .await
            .expect("续写幕宣布落得了格");
    }
    let second = call.take().expect("续写幕的交接件");

    assert_ne!(first.id, second.id, "幕锚进了逻辑 id，两幕才是两笔账");
    let all = h.facts.range(Seq::new(0));
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
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    // 上一段生命的遗留：宣布过、没派出，等它的 future 已经不在了。
    let orphan = Call::new("turn", "turn-0-d0-r0", 1);
    port::announce(&*h.store, &orphan, h.turn.turn_no, h.turn.user_seq).expect("遗留请求入格");

    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_dispatch_turn(&mut step)
            .await
            .expect("判死不算执行体失败");
    }

    let all = h.facts.range(Seq::new(0));
    let failed: Vec<&Event> = all.iter().filter(|e| e.kind == EVENT_FAILED).collect();
    assert_eq!(failed.len(), 1, "遗留请求被判死一次：{all:?}");
    assert_eq!(failed[0].payload["id"], "turn-0-d0-r0");
    assert_eq!(failed[0].payload["why"], "no_pending_round");
    assert!(
        !all.iter().any(|e| e.kind == EVENT_SENT),
        "判死不是派出——一条 `sent` 都不该有"
    );
    assert!(interrupted.is_none(), "没派出去就没有幕的结局");
}

/// 有请求、却没有输入那一半 ⇒ 也判死（发一条空请求比不派更坏）。
#[tokio::test]
async fn a_dispatch_without_inputs_kills_the_request_instead_of_firing_it() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    let announced = Call::new("turn", "turn-0-d0-r0", 1);
    port::announce(&*h.store, &announced, h.turn.turn_no, h.turn.user_seq).expect("请求入格");

    let mut ready = None;
    let mut call = Some(announced);
    let mut interrupted = None;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_dispatch_turn(&mut step)
            .await
            .expect("判死不算执行体失败");
    }

    let all = h.facts.range(Seq::new(0));
    let failed: Vec<&Event> = all.iter().filter(|e| e.kind == EVENT_FAILED).collect();
    assert_eq!(failed.len(), 1, "{all:?}");
    assert_eq!(failed[0].payload["why"], "no_prepared_inputs");
    assert!(
        !all.iter().any(|e| e.kind == EVENT_SENT),
        "一条空请求都没发"
    );
    assert!(interrupted.is_none());
}

/// 没有交接件、也没有遗留请求 ⇒ 一格不写（判死是**有对象才动**）。
#[tokio::test]
async fn a_dispatch_without_a_handover_and_nothing_dangling_writes_nothing() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    let before = h.facts.range(Seq::new(0)).len();

    let mut ready = None;
    let mut call = None;
    let mut interrupted = None;
    {
        let mut step = h.step(&mut ready, &mut call, &mut interrupted);
        run_dispatch_turn(&mut step)
            .await
            .expect("无事可做也是成功");
    }

    assert_eq!(
        h.facts.range(Seq::new(0)).len(),
        before,
        "没有对象可判死时不许造账"
    );
    assert!(interrupted.is_none());
}

// ── 停机：不欠账就一格不动 ───────────────────────────────────────────────

/// 本幕的三笔账都走完（准备 → 请求 → 派出 → 应答）⇒ 外环立刻收工。
///
/// 这条钉的是 `drive_stage` 的**停机判据**：选不出可还的债就返回，不拿假成功
/// 收场，也不顺手再宣布一笔。
#[tokio::test]
async fn drive_stage_stops_when_the_stage_owes_nothing() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();

    // 手工把这一幕走完：准备 → 宣布 → 派出 → 应答。
    land(
        &*h.store,
        h.turn.turn_no,
        h.turn.user_seq,
        "system",
        &[StageFact::ran(StageName::Inputs, 0).with_result("ready")],
    )
    .expect("准备格入格");
    let call = Call::new("turn", "turn-0-done", 1);
    port::announce(&*h.store, &call, h.turn.turn_no, h.turn.user_seq).expect("请求入格");
    port::begin(
        &*h.store,
        PORTS,
        Dispatch {
            call: &call,
            turn: h.turn.turn_no,
            produced_by: h.turn.user_seq,
            row: "dispatch-turn",
            now: h.turn.now_ms,
            awaited: false,
        },
    )
    .expect("派出入格");
    port::succeed(&*h.store, &call, h.turn.turn_no, h.turn.user_seq).expect("应答入格");
    // 收束格：一幕的终点。外环的停机判据读它（`stage_closed`），而没有它的「三笔账
    // 走完」只是半幕——环停下来之后本函数会按事实判一次「幕关了没有」。
    h.store
        .append(
            Event::pending(
                "final-0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                h.turn.turn_no,
                "assistant",
            )
            .with_produced_by(h.turn.user_seq)
            .with_payload(serde_json::json!({"text": "答完了", "reason": "completed"})),
        )
        .expect("收束格入格");
    let before = h.facts.range(Seq::new(0)).len();

    let out = drive_stage(StageDrive {
        orchestrator: &h.orchestrator,
        orchestrator_arc: &h.orchestrator,
        ctx: &h.ctx,
        sink: &h.sink,
        context: &mut h.context,
        turn: &mut h.turn,
        req: &h.req,
        user_text: "你好",
    })
    .await
    .expect("不欠账不是错误");

    assert!(
        out.interrupted.is_none(),
        "本幕跑完了：没有中断、也没有再准备一次"
    );
    assert_eq!(
        h.facts.range(Seq::new(0)).len(),
        before,
        "不欠账的一幕一格都不该多"
    );
    let all = h.facts.range(Seq::new(0));
    assert_eq!(
        turn_requests(&all).len(),
        1,
        "停机不是「再宣布一笔」：请求仍只有手工那一笔"
    );
}
