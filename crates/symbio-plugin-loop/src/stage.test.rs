//! `stage` 的单元测试——一幕的直线（幕首 → 轮首 → 接缝 + 内环 → 收好产出）。
//!
//! ## 这些用例钉的是哪几件事
//!
//! 1. **「备输入」落的是一格阶段事实**（`turn.stage{stage:"inputs"}`，`result:"ready"`）
//!    ——「这一幕跑过了」不只好是函数返回值；
//! 2. **定稿答话轮在轮首收束**：那一幕落 `result:"closed"`、返回 `Ok(None)`
//!    （否则一个已经答完的轮会被再答一次）；
//! 3. **重试的新支在备输入之后登记**：一条 `turn.branch`（`into` 用这一幕才存在的新根
//!    id），且 `retry_of` 取走即消费——续写幕不会再开一支。
//!
//! ## 为什么这里跑整幕
//!
//! 备输入不再是独立入口（`prepare` 随 `turn` 端口退场，`D-222`：幕是直线，不是
//! 迭代器）。要问「那一格落在哪一段」只能跑一幕——但跑幕不等于跑真模型：本文件的
//! 假 provider 只答一句话、不做工具轮，判据仍集中在**事实面**（哪几格落了、载荷
//! 是什么），与本文件成形之前同一组断言。工具轮与流式的判据在 `turn_exec.test.rs`。
//!
//! 判决 / 措辞 / 压缩三条产线都关掉：这些用例问的是幕的**骨架**，不是判决判成什么。
//! 关掉之后轮首只剩「召回 + 备输入出口」，重演的是最小的一条直线。

use std::sync::Arc;

use async_trait::async_trait;

use super::super::chat_loop::drive::begin_drive;
use super::super::chat_loop::state::{SessionContext, TurnExit, TurnRequest, TurnState};
use super::super::chat_loop::AnsweredTurn;
use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};
use super::run_stage;
use symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use symbio_core::principal::{Principal, PrincipalKind};
use symbio_core::{
    CapabilityMeta, Event, EventStore, ExecEventSink, ExecTranscriptWriter, ModelProvider,
    PluginError, PluginInvokeRequest, PluginSimpleRequest, Seq, Store, TurnOutput,
};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_TURN_BRANCH};

// ── 夹具 ────────────────────────────────────────────────────────────────

/// 帧口：外环不上线，所以只需要一个能塞进 sink 的空口。
struct NoopSink;

#[async_trait]
impl ExecTranscriptWriter for NoopSink {
    async fn apply(&self, _m: ChatNode) {}
}

/// 假 provider：**每幕答一句**。
///
/// 跑一幕本来就要走一次模型往返（`drive` 的 `announce-model` → `dispatch-model`），
/// 一个会失败的假件只能证明「幕在中途失败了」，证不了「幕跑完了」。这里答一句
/// 定死的话：本文件要的是整幕走到收束，正文内容与判据无关。
struct ReplyModel;

#[async_trait]
impl ModelProvider for ReplyModel {
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
    async fn round_trip(
        &self,
        _system: &str,
        _messages: &[ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Ok(TurnOutput {
            text: "好".into(),
            ..Default::default()
        })
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

/// 拆开的夹具：`run_stage` 要同时借到 `&mut context` / `&mut turn` 与
/// `&orchestrator`，而同一个结构体的字段分两次借（一次可变一次不可变）是不允许的，
/// 故先拆开。
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
    /// 判决 / 措辞 / 压缩**都关掉**：这些用例问的是幕的骨架，不是判决判成什么。
    /// 关掉之后 `policy.classify` / `policy.compress` 都不开账，轮首只剩召回与
    /// 备输入的出口。
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
            provider: Arc::new(ReplyModel),
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

    /// 跑一幕。
    async fn run(
        &mut self,
        user_text: &str,
    ) -> Result<Option<TurnExit>, super::super::turn_exec::ActorError> {
        run_stage(super::Stage {
            orchestrator: &self.orchestrator,
            orchestrator_arc: &self.orchestrator,
            ctx: &self.ctx,
            sink: &self.sink,
            context: &mut self.context,
            turn: &mut self.turn,
            req: &self.req,
            user_text,
        })
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

// ── 备输入：落出口、把一幕跑完 ───────────────────────────────────────────

/// 备输入落的是一格 `inputs` 阶段事实，随后这一幕照常跑完（收束格落格）。
#[tokio::test]
async fn run_stage_lands_the_inputs_stage_and_runs_the_turn() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();

    let exit = h.run("你好").await.expect("幕跑得完");
    assert!(exit.is_none(), "正常一幕没有中断结局：{exit:?}");

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("备输入的出口进了日志");
    assert_eq!(landed.payload["result"], "ready");
    assert_eq!(landed.payload["round"], 0);
    assert_eq!(landed.turn, h.turn.turn_no);

    assert!(
        all.iter().any(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL),
        "这一幕跑到了收束：收束格在"
    );
}

/// 重试开出的**新支在备输入之后登记**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 钉三件事，缺一件整套分支就读不对：
/// - `into` 用的是**这一幕才存在的新根 id**（`retry_of` 只是寄存的旧根，新根要到幕首
///   定下才有；早一格落它就只能按轮号归组，而一幕一个根、幕间续写会让同轮开出多个根）；
/// - 它是断言类事实 ⇒ 溯源指向本轮用户格（I2，缺了它整条 WAL 重放会被判违规）；
/// - `take()` 即消费 ⇒ **第二幕不再另开一支**（否则幕间续写会凭空多出一个候选，
///   用户看到 `2/3` 却只重试过一次）。
#[tokio::test]
async fn run_stage_lands_one_branch_fact_for_a_retry_and_consumes_the_registration() {
    let mut h = Harness::new(vec![user_node("m1", "你好")]);
    h.open();
    h.turn.retry_of = Some("t-old".to_string());

    h.run("你好").await.expect("幕跑得完");
    assert!(
        h.turn.retry_of.is_none(),
        "寄存的旧根被取走：同一幕不会落两条分支格"
    );
    let root_id = h
        .turn
        .branch_root
        .clone()
        .expect("收尾记下了本幕的根（组内标注要用）");

    let landed = branch_facts(&h);
    assert_eq!(landed.len(), 1, "一次重试只开一支：{landed:?}");
    let branch = &landed[0];
    assert_eq!(branch.payload["of"], "t-old");
    assert_eq!(branch.payload["into"], root_id.as_str());
    assert_eq!(
        branch.produced_by,
        Some(h.turn.user_seq),
        "分支格是断言类：溯源必须指向本轮用户格"
    );
    assert!(!branch.actor.is_empty(), "actor 取本轮主体，不留空");

    // 续写幕：`retry_of` 已被取走 ⇒ 再走一次登记是空操作，事实面不多第二支。
    //
    // 不真跑第二幕：同一轮已经收束，运行器不会再收束第二条（那是「一轮至多一条
    // 收束格」本身），第二幕跑不到「登记之后」那几格。这里直接问那个登记口——
    // 它正是每一幕都会调的那一处。
    h.turn.drive_seq += 100;
    super::super::turn_exec::land_retry_branch(
        h.store.as_ref(),
        h.ctx.as_ref(),
        &mut h.turn,
        "r-next",
    )
    .expect("登记是空操作");
    assert_eq!(branch_facts(&h).len(), 1, "幕间续写不再开第二支");
}

/// 定稿答话轮：轮首就地收束 ⇒ 落 `result:"closed"`、**不**跑轮内环。
///
/// 这一格是「这一幕不进内环」的唯一依据：`Ok(None)` 就是它。只落 `ready` 的话，收束的
/// 那一幕在事实面上与「正常跑完」同形——一个已经答完的轮会再答一次。
#[tokio::test]
async fn run_stage_lands_closed_when_the_turn_answers_in_place() {
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

    let exit = h.run("你好").await.expect("收束不是失败");
    assert!(exit.is_none(), "收束的一幕正常返回（流向读事实）");

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("收束也要落格（否则与「还没备」同形）");
    assert_eq!(landed.payload["result"], "closed");
    // 答话轮只发一次「措辞」之外的往返都没有：模型假装件一次都没被问。
    assert!(
        all.iter()
            .filter(|e| e.kind == "request" && e.payload["port"] == "model")
            .count()
            == 0,
        "定稿答话轮不进轮内环，零模型往返"
    );
}
