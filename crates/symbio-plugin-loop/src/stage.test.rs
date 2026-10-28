//! `stage` 的单元测试——一幕那三段直线里**备输入**那一段落的格。
//!
//! ## 这些用例钉的是哪几件事
//!
//! 1. **准备落的是一格阶段事实**（`turn.stage{stage:"inputs"}`）——「做完了」不只好是
//!    函数返回值；
//! 2. **定稿答话轮在准备段收束**：那一幕落 `result:"closed"`、不交输入（否则一个
//!    已经答完的轮会被再答一次）；
//! 3. **重试的新支在准备段登记**：一条 `turn.branch`（`into` 用这一幕才存在的新根 id），
//!    且 `retry_of` 取走即消费——续写幕不会再开一支。
//!
//! ## 为什么这里不跑一整个幕
//!
//! 跑一轮的执行体是一整幕（`execute_turn`），它的判据在 `turn_exec.test.rs`。这里要
//! 问的是**准备段落的格**，跑真模型会把两组判据混成一堆互相解释不了的断言。假
//! provider 因此只会被问「有没有人调我」——它一被调用就返回错，用例当场红。
//!
//! 「宣布 / 派出」两个环步已随 `turn` 端口退场（`D-222`：幕是直线，不是迭代器），
//! 所以这里没有「宣布落的是一条 `request(port=turn)`」这类用例——那断言的是一个
//! 被删除的形状（`SP-033` §1：改用例，不改形状）。

use std::sync::Arc;

use async_trait::async_trait;

use super::super::chat_loop::drive::begin_drive;
use super::super::chat_loop::inputs::TurnInputs;
use super::super::chat_loop::state::{SessionContext, TurnRequest, TurnState};
use super::super::chat_loop::AnsweredTurn;
use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};
use super::prepare;
use symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use symbio_core::principal::{Principal, PrincipalKind};
use symbio_core::{
    CapabilityMeta, Event, EventStore, ExecEventSink, ExecTranscriptWriter, ModelProvider,
    PluginError, PluginInvokeRequest, PluginSimpleRequest, Seq, Store, TurnOutput, EVENT_REQUEST,
};
use symbio_plugin_api::events::EVENT_TURN_BRANCH;

// ── 夹具 ────────────────────────────────────────────────────────────────

/// 帧口：外环不上线，所以只需要一个能塞进 sink 的空口。
struct NoopSink;

#[async_trait]
impl ExecTranscriptWriter for NoopSink {
    async fn apply(&self, _m: ChatNode) {}
}

/// 假 provider：**只会被问「有没有人调我」**。
///
/// 一被调用就返回错——这样「准备段偷偷跑了整幕」会是一条红，而不是一条看起来
/// 正常的通过。
struct NoModel;

#[async_trait]
impl ModelProvider for NoModel {
    fn provider_id(&self) -> &str {
        "stage-mock"
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
        Err(PluginError::InternalError("准备段不该调模型".into()))
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

/// 拆开的夹具：`prepare` 要同时借到 `&mut context` 与 `&orchestrator`，
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
    /// 判决与措辞**都关掉**：这些用例问的是准备段落的格本身，不是判决判成什么。
    /// 关掉之后 `policy.classify` / `policy.compress` 都不开账，准备段只剩
    /// 「收集 + 召回 + 落出口」。
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

    /// 跑一次准备段。
    async fn prepare(&mut self) -> Result<Option<TurnInputs>, super::super::turn_exec::ActorError> {
        prepare(
            &self.orchestrator,
            &self.ctx,
            &self.sink,
            &mut self.context,
            &mut self.turn,
            &self.req,
            &self.store,
        )
        .await
    }
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

/// 准备落的是一格 `inputs` 阶段事实，输入交回给幕的下一段。
#[tokio::test]
async fn prepare_lands_the_inputs_stage_and_hands_the_inputs_over() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();

    let inputs = h
        .prepare()
        .await
        .expect("准备落得了格")
        .expect("这一幕要跑");

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("准备段的出口进了日志");
    assert_eq!(landed.payload["result"], "ready");
    assert_eq!(landed.payload["round"], 0);
    assert_eq!(landed.turn, h.turn.turn_no);

    assert!(!inputs.root_id.is_empty(), "根骨架已分配");
    // 幕不再是端口：备输入**不宣布任何请求**（`D-222` §一：`turn` 是重入，不是往返）。
    assert!(
        !all.iter().any(|e| e.kind == EVENT_REQUEST),
        "备输入不落任何 `request`：`turn` 端口已退场"
    );
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

    let new_root = h
        .prepare()
        .await
        .expect("准备落得了格")
        .expect("这一幕要跑")
        .root_id;
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
    h.prepare().await.expect("续写幕准备落得了格");
    assert_eq!(branch_facts(&h).len(), 1, "幕间续写不再开第二支");
}

/// 定稿答话轮：准备段就地收束 ⇒ 落 `result:"closed"`、**不**交输入。
///
/// 这一格是「这一幕不跑」的唯一依据：`Ok(None)` 就是它。只落 `ready` 的话，收束的
/// 那一幕在事实面上与「还没准备」同形，一段返回输入、一段照常跑一轮——一个已经答完
/// 的轮会再答一次。
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

    let inputs = h.prepare().await.expect("收束不是失败");
    assert!(inputs.is_none(), "收束的一幕没有输入");

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("收束也要落格（否则与「还没准备」同形）");
    assert_eq!(landed.payload["result"], "closed");
    assert!(
        !all.iter().any(|e| e.kind == EVENT_REQUEST),
        "收束的一幕不宣布任何往返"
    );
}
