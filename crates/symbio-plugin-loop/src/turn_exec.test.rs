//! v2 执行器验收——无工具轮的四条硬契约：
//!
//! 1. **事实原生入格**：用户格 + final 格落在本会话的 v2 WAL，不变量全绿；
//! 2. **流式回同一出口**：生成增量经 UiBridge 变成 UI 帧（首片快照
//!    建节点 + 后续窄帧），帧语义与 model 插件直发同构；
//! 3. **兜底也是一句话**（I3）：生成失败落 fallback 格后上抛——用户侧
//!    失败呈现走 `Failed` 出口，网格里不缺格；
//! 4. **中止不是失败**：生成被中止时**照落收束格**（`chat.assistant.final`，
//!    载荷 `status: "aborted"` + `terminated`），上抛 `Aborted` 走独立出口——
//!    不呈现「失败」也不进兜底分子；半截产出随出口带走（`partial`），由消费
//!    循环收尾入格。中止与失败的差别在**类型上**可见，不靠错误
//!    文本猜。

use crate::control_decl::guard_vocab;
use std::sync::{Arc, Mutex};

use super::*;
use symbio_core::chat_node as cm;
use symbio_core::{
    check_all, run_ops, CapabilityMeta, ConsolidateRequest, Entity, Event, ExecTranscriptWriter,
    Frozen, PluginSimpleRequest, Seq, Store, Verb,
};
use symbio_core::{
    Capability, CapabilityRow, EventStore, ExecEnv, PluginError, PluginInvokeRequestExt, Registry,
    CAPABILITY_VISITOR,
};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use symbio_providers::DefaultToolVisitor;

use crate::control_decl::{CHAT_VOCAB, EVENT_SESSION_CONFIG, MESSAGE_VOCAB};

use super::super::session_port::SessionPort;
use super::super::test_support::{default_settings, loop_session, turn_spec, FakeSessionPort};

/// 身份写 `principal`（D-147）：夹具模仿的是**桥之后**的节点面（生产里
/// `assemble` 的发射任务把会话主体写进每一帧），`role` 只是反序列化落点。
fn principal_for(role: cm::NodeRole) -> symbio_core::principal::Principal {
    use symbio_core::principal::Principal;
    match role {
        cm::NodeRole::User => Principal::from_legacy(symbio_core::PRINCIPAL_USER),
        cm::NodeRole::Assistant => Principal::from_legacy("session/main"),
        cm::NodeRole::Tool => Principal::from_legacy("legacy/unknown"),
        cm::NodeRole::System => Principal::from_legacy("system/local"),
    }
}

/// 测试请求上下文（工具轮才需要能力注册表；无工具轮一个空信封足够）。
fn test_ctx() -> Arc<dyn symbio_core::PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 带**注册面**的请求上下文：把 `tools` 注册进 `CAPABILITY_VISITOR`。
///
/// 三道按名闸门（淘汰 / 承诺记账 / 任务表）的管辖判据现在从注册面读自声明位，
/// ctx 里没有注册面 = 「没有任何工具声明过任何位」。生产里这个注册面由会话
/// 插件的能力收集装上（`capabilities.rs`），测试照同一形状喂——`tools` 传什么，
/// 注册面就是什么，两边不会漂。
async fn test_ctx_declaring(tools: &[CapabilityMeta]) -> Arc<dyn symbio_core::PluginInvokeRequest> {
    let ctx = test_ctx();
    let visitor = Arc::new(DefaultToolVisitor::new());
    for t in tools {
        visitor
            .put(CapabilityRow::tool(
                Arc::new(DeclOnlyTool(t.clone())) as Arc<dyn Capability>
            ))
            .await;
    }
    ctx.set(CAPABILITY_VISITOR, visitor as Arc<dyn Registry>);
    ctx
}

/// 只带元数据的假工具：注册面只读 `meta()`，执行永不到达（这些用例的调用都
/// 止步于闸门 / 诚实失败，不真的 invoke）。
struct DeclOnlyTool(CapabilityMeta);

#[async_trait::async_trait]
impl Capability for DeclOnlyTool {
    fn meta(&self) -> CapabilityMeta {
        self.0.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::Value::Null)
    }
}

/// **轮次事实**（这一轮说了什么 / 跑出了什么）——记忆与学习是**派生副作用**，
/// 不在其中（见 `full_turn_writes_no_memory_encoding_from_the_host`）。
///
/// 断言轮次事实时先按它筛一遍：网格里同时住着两类事件，用「总数等于几」去断言
/// 会把两类混在一起——派生素化多写一条就假红，少写一条又假绿。
fn turn_facts(e: &Event) -> bool {
    matches!(
        e.kind.as_str(),
        EVENT_USER_MESSAGE
            | EVENT_CHAT_ASSISTANT_FINAL
            | EVENT_CHAT_ASSISTANT_FALLBACK
            | symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_PROGRESS
            | symbio_plugin_api::events::EVENT_ARTIFACT_ADDED
    )
}

/// 无工具轮的入参包（本文件的三个用例都是这一形态）。
fn tool_free_req<'a>(
    fx: &'a Fixture,
    provider: Arc<dyn ModelProvider>,
    sink: &'a symbio_core::ExecEventSink,
    abort: symbio_core::ExecAbortSignal,
    turn_no: u64,
    user_text: &'a str,
) -> ExecTurn<'a> {
    let user_seq = seed_user(fx, turn_no, user_text);
    ExecTurn {
        turn_no,
        user_seq,
        drive_seq: user_seq,
        user_text,
        ..exec_base(&fx.session, provider, sink, abort)
    }
}

/// **已开未收束**那一轮的入参包：**不新开用户格**，溯源锚由调用方直给。
///
/// 唤醒幕的身份不再由入参携带（d：旁路退役）：它落在**事实**上——
/// `turn.stage{stage:"resume"}` 是本幕的锚，`turn_head` 据此判「本幕是续写幕」。
fn open_turn_req<'a>(
    fx: &'a Fixture,
    provider: Arc<dyn ModelProvider>,
    sink: &'a symbio_core::ExecEventSink,
    abort: symbio_core::ExecAbortSignal,
    turn_no: u64,
    user_seq: u64,
) -> ExecTurn<'a> {
    ExecTurn {
        turn_no,
        user_seq,
        drive_seq: user_seq,
        ..exec_base(&fx.session, provider, sink, abort)
    }
}

/// 落**恢复阶段事实**（唤醒幕的锚），交回它的 seq。
///
/// 生产里这一格由 `chat_loop::begin_drive` 落（d 之后）：续写的身份
/// 在**事实**上，不在入参里。幕锚从用户格挪到这一条 ⇒ 幕锚轮号回到被续写的那
/// 一轮（哪怕它已被后来的轮次归档），`turn_head.resumed = true`。
fn land_resume_anchor(fx: &Fixture, turn_no: u64, user_seq: u64, kind: &str) -> u64 {
    let seqs = super::super::stage_events::land_with_seqs(
        fx.facts.as_ref(),
        turn_no,
        user_seq,
        "system",
        &[super::super::stage_events::StageFact::resume(0, kind, "1")],
    )
    .expect("恢复阶段事实落格");
    seqs.first().copied().unwrap_or(user_seq)
}

/// 测试侧的交付包：**一幕的幕首入参**（形状与生产 [`super::ExecTurn`] 同一份）。
///
/// 生产里幕首之后只有**一个环**（`D-222` 裁决 5）：轮首四行、接缝、汇报与轮内各行
/// 同住一张绑定表，先后由 `prio` 说。接缝读的寄存物因此不再另开一条通道递进来——
/// 它们住在**本轮状态**与**会话上下文**上（`recall_view` / `opening_text` /
/// `consolidated`），由环上的动作自己取走。本夹具
/// 替幕首把那一片备好，于是轮首那几行没开账时这些寄存是 `None`，开账时由执行体覆盖。
///
/// **本地同名结构遮蔽了 `super::*` 里的 `ExecTurn`**——五个 struct 字面量因此
/// 一字不改（`SP-033` §1：改用例，不改形状）。
struct ExecTurn<'a> {
    session: &'a Arc<LoopSession>,
    provider: Arc<dyn ModelProvider>,
    parent: Option<Arc<dyn Plugin>>,
    ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    system_prompt: &'a str,
    abort: symbio_core::ExecAbortSignal,
    root_id: &'a str,
    sink: &'a symbio_core::ExecEventSink,
    user_text: &'a str,
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    window_turns: u64,
    tools: &'a [CapabilityMeta],
    recalled: Option<&'a super::super::recall::RecallView>,
    /// 快路作答的正文：生产里由技能行的 `RowStep::Answer` 让宿主经
    /// `TurnSession::set_reflex` 寄进草稿（D-260）。夹具在同一处模拟那一半。
    reflex_text: Option<&'a str>,
    consolidation: Option<&'a [ChatNode]>,
    supplements: Option<super::SupplementFn>,
    supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    report: Option<super::super::report_exec::ReportPort>,
    opening_text: Option<&'a str>,
    opening_dialog: Option<&'a super::super::turn_runner::DialogFacts>,
    presentation_tree: symbio_core::node_build::PresentationTree,
}

/// 编排器那份转写写口：本夹具不测压缩改道，交一个丢弃口。
struct DiscardWriter;

#[async_trait::async_trait]
impl symbio_core::ExecTranscriptWriter for DiscardWriter {
    async fn apply(&self, _m: cm::ChatNode) {}
}

/// 一幕的执行：装配 → 开轮 → **一个环**（轮首 / 接缝 / 轮内）→ 归类 / 落格。
///
/// 与生产 [`super::drive`] 的 `Control` 入口只差两处：**幕首**那几行（用户格 /
/// 配置格 / 轮次格由本文件的 `seed_*` 直给）与**幕末**的落库与镜像回灌。环里那一段
/// 是同一个 `drive`、同一张绑定表——轮首四行与接缝不再由夹具代跑，而是照旧欠账、
/// 照旧由 `prio` 选中（没开账的行不会被选中，判据因此仍落在本文件那几条契约上）。
async fn drive_turn(req: ExecTurn<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    let ExecTurn {
        session,
        provider,
        parent,
        ctx,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        turn_no,
        user_seq,
        drive_seq,
        window_turns,
        tools,
        recalled,
        reflex_text,
        consolidation,
        supplements,
        supplemental_no,
        report,
        opening_text,
        opening_dialog,
        presentation_tree,
    } = req;
    // 幕锚的「现在」与生产同一个读数口（派发器与视图都读它，不是夹具编的一个数）。
    let facts = session
        .facts()
        .map(|store| store.range(Seq::new(0)))
        .unwrap_or_default();
    let now_ms = super::super::loop_rows::wake_now_ms(&facts, drive_seq);

    // 环要借的那一片可控状态：生产由 `turn_exec::drive` 的幕首备好，这里同一形状。
    // `input_utterance` 留 `None`：Q1 的委派判定因此不下结论（与退役前那条旁路同形）。
    let orchestrator = Arc::new(super::super::chat_loop::state::ChatOrchestrator {
        provider: provider.clone(),
        parent: parent.clone(),
        context_limit: 8192,
        session: session.clone(),
        writer: Arc::new(DiscardWriter),
        compose_enabled: false,
        opening_enabled: false,
        presentation_tree,
    });
    let mut context = super::super::chat_loop::state::SessionContext {
        messages: Vec::new(),
        session: session.clone(),
        consolidated: consolidation.map(|c| c.to_vec()),
    };
    let mut turn = super::super::chat_loop::state::TurnState {
        abort: abort.clone(),
        turn_no,
        user_seq,
        drive_seq,
        now_ms,
        recall_view: recalled.cloned(),
        opening_text: opening_text.map(String::from),
        opening_dialog: opening_dialog.cloned(),
        ..Default::default()
    };

    let turn_req = super::super::chat_loop::state::TurnRequest {
        auto_compress: false,
        load_history: true,
        system_prompt: None,
    };
    // 请求句柄在装配时被搬进装配束，而轮首那一片也要它（分发方与它同源）——先克隆。
    let request_ctx = ctx.clone();
    let (assembled, emitter) = super::assemble_turn(super::ExecTurn {
        session,
        provider,
        parent,
        ctx,
        system_prompt,
        abort,
        root_id,
        fresh_root: true,
        sink,
        user_text,
        turn_no,
        user_seq,
        drive_seq,
        window_turns,
        tools,
        supplements,
        report,
        supplemental_no,
        presentation_tree,
    })
    .await?;
    let open = super::open_turn(&assembled).await?;
    // 幕首之后那一半（分发方 / 汇报落地面 / 宿主面）与生产同一个构造点：夹具自己备的
    // 只是幕首（用户格 / 配置格由本文件的 `seed_*` 直给）。
    let mut stage = super::stage_runtime(assembled, emitter, open);
    stage.set_reflex_for_test(reflex_text.map(str::to_string));
    let head = stage.head_parts(
        orchestrator.clone(),
        request_ctx,
        &mut context,
        &mut turn,
        &turn_req,
    );
    match super::drive(
        super::DriveStart::Ready(Box::new(super::ReadyStage { stage, head })),
        super::super::loop_rows::ACTORS,
    )
    .await?
    {
        // 环跑到收束格出现：归类（中止 / 授权拒 / 正常）与落格按本文件要验的
        // 那几条契约做——生产的镜像并入与封根不在夹具里（`stage.test.rs` 钉）。
        super::DriveEnd::Running(running) => {
            match super::classify_turn(*running, turn.recall_view.as_ref()).await? {
                super::Classified::Finished(v) => v,
                super::Classified::Settled(settled) => super::persist_turn(settled),
            }
        }
        // 判成「就此答话」/ 幕间中止都是控制面才有的两路：本夹具的一轮不会走到
        // （它不带裁决寄存、也没有第二幕）。按失败报出，不静默交回一个空结果。
        super::DriveEnd::Answered => Err(TurnExecExit::Failed(PluginError::InternalError(
            "夹具的一轮不会判成答话".into(),
        ))),
        super::DriveEnd::Boundary | super::DriveEnd::Completed | super::DriveEnd::Bare(_) => {
            Err(TurnExecExit::Failed(PluginError::InternalError(
                "夹具的一轮只走收束一路（环交回了别的结局）".into(),
            )))
        }
    }
}

fn exec_base<'a>(
    session: &'a Arc<LoopSession>,
    provider: Arc<dyn ModelProvider>,
    sink: &'a symbio_core::ExecEventSink,
    abort: symbio_core::ExecAbortSignal,
) -> ExecTurn<'a> {
    ExecTurn {
        session,
        provider,
        parent: None,
        ctx: test_ctx(),
        system_prompt: "system",
        abort,
        root_id: "r-1",
        sink,
        user_text: "你好",
        turn_no: 0,
        user_seq: 0,
        drive_seq: 0,
        window_turns: 6,
        tools: &[],
        recalled: None,
        reflex_text: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        report: None,
        // 开场白：本夹具不产生（`Escalate` 首响由 chat_loop 递入）。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    }
}

/// 测试侧先落用户格：生产由 `chat_loop::drive` 在判决之前落。
fn seed_user(fx: &Fixture, turn: u64, text: &str) -> u64 {
    fx.facts
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                "user",
            )
            .with_payload(serde_json::json!({ "text": text })),
        )
        .expect("用户格入格")
        .value()
}

/// 生产形状的配置格（`chat_loop/drive.rs` 的 `begin_drive` 落的那一份）。
///
/// 本文件的用例大多直调 `drive_turn`，绕过了开幕那一步；只要有一行读
/// `session.config`（如 `report_due` 的开头），配置格就得自己落——否则那个开关
/// 恒为 false，行永远不开账，用例会红在一个与它想验的东西无关的地方。
fn seed_config(fx: &Fixture, turn: u64, user_seq: u64) {
    fx.facts
        .append(
            Event::pending(
                format!("cfg-{turn}-{user_seq}"),
                EVENT_SESSION_CONFIG,
                Entity::Turn,
                Verb::Asserted,
                turn,
                "system",
            )
            .with_produced_by(user_seq)
            .with_payload(serde_json::json!({
                "classify_enabled": false,
                "auto_compress": false,
                "progress_enabled": true,
                "progress_interval_ms": 0,
                "progress_min_rounds": 0,
                "progress_max_per_turn": 2,
                "is_persistent": true,
            })),
        )
        .expect("配置格入格");
}

/// 帧收集口（测试的 ExecEventSink 出口）。
struct CollectingFrames(Mutex<Vec<cm::ChatNode>>);

#[async_trait::async_trait]
impl ExecTranscriptWriter for CollectingFrames {
    async fn apply(&self, m: cm::ChatNode) {
        self.0.lock().unwrap().push(m);
    }
}

/// 补充成为事实、并在**下一轮**仍可见（插件层那一半判据）。
///
/// ## 为什么这条在插件层而不在 core
///
/// core 只提供「工具循环里什么时候问一次」这个挂点（`TurnInput::inject`）；
/// 「什么是补充」「它怎么落成 `turn.supplemented`」全在 `crates/symbio-plugin-session`（收件箱、
/// `merge_supplements`、`EventWalStore`）。core 那侧的判据是
/// `injected_messages_reach_the_next_request_of_the_same_turn`（只验「进了本轮请求」）。
///
/// ## 三段各只能证明一件事
///
/// - **落格**（WAL 有 `turn.supplemented` + 溯源 + `count`）：它成为了事实；
/// - **回读下一次请求**：它在**本轮**就对模型可见（用户就是在这轮说的话）；
/// - **重开 WAL 再投影**：它在**下一轮**仍可见——这才是原始症状
///   （只进消息列表、不进事实网格的补充，下一轮就从模型眼前消失）。
///
/// ## 反向自检
///
/// 把闭包里的 `store.append` 去掉 ⇒ 第 1、3 段红（网格里没有它）；
/// 把 `store.append` 保留但返回空 vec ⇒ 第 2 段红（本轮请求里没有它）。
#[tokio::test]
async fn supplements_become_facts_visible_next_turn() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::ToolThenAnswer);

    // 假的抽干口：第一次返回一批（**2 条合成 1 段**），之后空。
    //
    // 形状与生产一致（`DrainedSupplement`：正文 + 原条数）。用 2 条是因为「n 条
    // 合成 1 段」是真实场景，而 `count` 填错（比如填 1）该被照出来。
    // `Arc<dyn Fn>` 要求 `'static`，所以游标也得是 `Arc`（借用会在闭包逃逸时报错）。
    let fired = Arc::new(Mutex::new(false));
    let supplements = {
        let fired = fired.clone();
        Arc::new(move || {
            let fired = fired.clone();
            Box::pin(async move {
                let mut once = fired.lock().unwrap();
                if *once {
                    return None;
                }
                *once = true;
                Some(symbio_core::DrainedSupplement {
                    text: "顺便也看看 README".into(),
                    count: 2,
                    id: "sup-1".into(),
                    ids: vec!["sup-1".into(), "sup-2".into()],
                })
            })
                as std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = Option<symbio_core::DrainedSupplement>>
                            + Send,
                    >,
                >
        }) as super::SupplementFn
    };

    let user_seq = seed_user(&fx, 0, "读一下 a.md");
    drive_turn(ExecTurn {
        turn_no: 0,
        user_seq,
        drive_seq: user_seq,
        session: &fx.session,
        provider: Arc::new(provider),
        parent: None,
        ctx: test_ctx(),
        system_prompt: "system",
        abort: symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            category: None,
            examples: None,
            ..Default::default()
        }],
        recalled: None,
        reflex_text: None,
        consolidation: None,
        supplements: Some(supplements),
        supplemental_no: Default::default(),
        report: None,
        // 开场白：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    })
    .await
    .expect("工具轮执行成功");

    // ── ① 落格：事实轨里有 `turn.supplemented`，带溯源与 `count` ──────
    let snap = fx.facts.range(Seq::new(0));
    let sup = snap
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED)
        .unwrap_or_else(|| {
            panic!("补充必须落成事实（否则下一轮就消失——那正是轮内注入要消掉的失效）：{snap:?}")
        });
    assert_eq!(sup.entity, symbio_core::Entity::Turn, "落在 Turn 实体上");
    assert_eq!(sup.verb, symbio_core::Verb::Asserted, "断言类动词");
    assert_eq!(
        sup.payload.get("count").and_then(|v| v.as_u64()),
        Some(2),
        "原条数要原样保留（2 条合成 1 段）"
    );
    let user_seq = snap
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_USER_MESSAGE)
        .and_then(|e| e.seq)
        .map(|s| s.value())
        .expect("网格里有用户格");
    assert_eq!(
        sup.produced_by,
        Some(user_seq),
        "断言类事件必须带溯源（I2），且指向**本轮用户格**"
    );
    // ── ② 本轮可见：收尾轮的请求里带上了它，且是 role=user ──────────────
    let seen = prompts.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "工具轮 + 收尾轮：{seen:?}");
    assert!(
        !seen[0].iter().any(|(_, t)| t.contains("README")),
        "第一次请求还没有注入内容（那时抽干口还没被问）：{seen:?}"
    );
    assert!(
        seen[1]
            .iter()
            .any(|(r, t)| r == "user" && t.contains("README")),
        "收尾轮请求须带上补充，且是 role=user：{seen:?}"
    );

    // ── ③ 下一轮可见：从事实轨**整份史**再投影（不拿运行中的内存镜像糊弄） ──
    let projected = symbio_core::transcript(CHAT_VOCAB, MESSAGE_VOCAB)
        .apply(
            &fx.facts.range(Seq::new(0)),
            i64::MAX,
            symbio_core::Budget::generous(),
        )
        .value;
    assert!(
        projected
            .entries
            .iter()
            .any(|e| e.role == "user" && e.text.contains("README")),
        "补充必须在**下一轮**仍可见（从事实投影）：{projected:?}"
    );
}

/// 假 provider 的行为档：忠实流式 / 生成失败 / 中止 / 工具轮。
#[derive(Clone, Copy)]
enum Behavior {
    /// 像真实 SSE 一样经 `env` 发「快照 + 窄帧」，返回聚合全文。
    Faithful,
    /// 按生成失败返回（演练 I3 兜底格）。
    Fail,
    /// 按中止返回（演练「中止照落收束格」，本轮无半截产出）。
    Abort,
    /// 先流出半截思考与半截正文、再按中止返回——演练「半截产出随出口带走」。
    AbortAfterPartial,
    /// 首次请求回一个工具调用（附中途正文），第二次回正文——演练工具轮。
    ToolThenAnswer,
    /// 首次请求回一个 `agent_run` 工具调用，第二次回正文——演练**代际立约**
    /// 的入格通道（承诺出参经 `SessionDispatchPort::take_derived` 回到运行器）。
    ToolThenAgentRun,
}

/// 一次请求被记下来的形状：`(role, 正文)` 逐条（顺序即发送顺序）。
///
/// 提成别名不是洁癖——它出现在两个字段与一个返回值上，内联写要占一行长类型，
/// 而**读的人要的是「这是什么」不是「它是几层泛型嵌套」**。
type ReceivedMessages = Vec<Vec<(String, String)>>;

/// 忠实假 provider：行为由 [`Behavior`] 选定（同一传输形状，只换结局）。
struct FaithfulProvider {
    behavior: Behavior,
    /// 每次请求收到的**消息数组**（工具轮的验收要回读**下一次请求**）。
    ///
    /// 记的是 `drive_turn` 真正收到的 `&[ChatNode]` —— 即**线格式那一侧**，
    /// **全部消息**（不只 `first()`）：只取首条时「整段被塞进一条 user 消息」测不出来。
    prompts: Arc<Mutex<ReceivedMessages>>,
}

impl FaithfulProvider {
    fn new(behavior: Behavior) -> (Self, Arc<Mutex<ReceivedMessages>>) {
        let prompts: Arc<Mutex<ReceivedMessages>> = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                behavior,
                prompts: prompts.clone(),
            },
            prompts,
        )
    }
}

#[async_trait::async_trait]
impl ModelProvider for FaithfulProvider {
    fn provider_id(&self) -> &str {
        "faithful-mock"
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
        messages: &[cm::ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        env: &symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        // 记下本次请求的**全部消息**（角色 + 正文）——工具轮的硬契约是「工具结果
        // 进了下一次请求」，那条断言只能回读请求体（不变量/网格都证明不了）。
        //
        // ⚠️ 记**全部消息**，不只 `messages.first()`：只取首条时「工具结果是
        // `role=tool`」不可断（整段挤在一条 user 消息里也照样绿）。
        let received: Vec<(String, String)> = messages
            .iter()
            .map(|m| {
                // 身份走折叠读法（主体在先、残留格兜底）；`Debug` 会打成 `user`，
                // 断言要的就是这个裸词。
                let role = format!("{:?}", m.node_role().unwrap_or_default()).to_lowercase();
                let text = m.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
                (role, text)
            })
            .collect();
        let round = {
            let mut p = self.prompts.lock().unwrap();
            p.push(received);
            p.len()
        };
        match self.behavior {
            Behavior::Fail => return Err(PluginError::InternalError("模型断了".into())),
            Behavior::Abort => return Err(PluginError::Aborted),
            Behavior::AbortAfterPartial => {
                // 真实链路的半截形态：推理快照帧 + 正文快照帧先行（首片即快照），
                // 生成被中止在半句话上——帧进桥的累积，收束与出口按累积取半截。
                env.sink()
                    .emit(
                        cm::ChatNode {
                            id: "n-rea".into(),
                            parent_id: Some("r-1".into()),
                            principal: Some(principal_for(cm::NodeRole::Assistant)),
                            msg_type: Some(cm::NodeKind::Notice),
                            content: Some(cm::MessageContent::Text("这个名词需要先定义。".into())),
                            status: Some(cm::NodeStatus::Streaming),
                            ..Default::default()
                        }
                        .with_style_tag(cm::CHAT_STYLE_TAG_REASONING),
                    )
                    .await;
                env.sink()
                    .emit(cm::ChatNode {
                        id: "n-half".into(),
                        parent_id: Some("r-1".into()),
                        principal: Some(principal_for(cm::NodeRole::Assistant)),
                        msg_type: Some(cm::NodeKind::Body),
                        content: Some(cm::MessageContent::Text("让我想想这个名词".into())),
                        status: Some(cm::NodeStatus::Streaming),
                        ..Default::default()
                    })
                    .await;
                return Err(PluginError::Aborted);
            }
            Behavior::Faithful => {}
            Behavior::ToolThenAnswer => {
                if round == 1 {
                    // 中途正文：真实模型调工具前常先说一句。它必须被定格并切节点，
                    // 否则会与收尾正文粘成一条。
                    env.sink()
                        .emit(cm::ChatNode {
                            id: "n-tool".into(),
                            parent_id: Some("r-1".into()),
                            principal: Some(principal_for(cm::NodeRole::Assistant)),
                            msg_type: Some(cm::NodeKind::Body),
                            content: Some(cm::MessageContent::Text("我先查一下。".into())),
                            status: Some(cm::NodeStatus::Streaming),
                            ..Default::default()
                        })
                        .await;
                    return Ok(TurnOutput {
                        text: "我先查一下。".into(),
                        tool_calls: vec![symbio_core::TurnToolCallInfo {
                            id: Some("tc-1".into()),
                            wire_id: Some("call_1".into()),
                            name: Some("vdfs_read".into()),
                            arguments: serde_json::json!({ "path": "a.md" }),
                            parse_error: None,
                            overflow_notice: None,
                        }],
                        response_text_child_id: "n-tool".into(),
                        ..Default::default()
                    });
                }
            }
            Behavior::ToolThenAgentRun => {
                if round == 1 {
                    env.sink()
                        .emit(cm::ChatNode {
                            id: "n-agent".into(),
                            parent_id: Some("r-1".into()),
                            principal: Some(principal_for(cm::NodeRole::Assistant)),
                            msg_type: Some(cm::NodeKind::Body),
                            content: Some(cm::MessageContent::Text("我来委托。".into())),
                            status: Some(cm::NodeStatus::Streaming),
                            ..Default::default()
                        })
                        .await;
                    return Ok(TurnOutput {
                        text: "我来委托。".into(),
                        tool_calls: vec![symbio_core::TurnToolCallInfo {
                            id: Some("tc-agent".into()),
                            wire_id: Some("call_agent".into()),
                            name: Some("agent_run".into()),
                            arguments: serde_json::json!({
                                "agent_id": "reviewer",
                                "prompt": "让 reviewer 复查这段",
                            }),
                            parse_error: None,
                            overflow_notice: None,
                        }],
                        response_text_child_id: "n-agent".into(),
                        ..Default::default()
                    });
                }
            }
        }
        // 真实链路的帧形态：正文快照（Text × Streaming）先行，窄帧随后。
        env.sink()
            .emit(cm::ChatNode {
                id: "n-1".into(),
                parent_id: Some("r-1".into()),
                principal: Some(principal_for(cm::NodeRole::Assistant)),
                msg_type: Some(cm::NodeKind::Body),
                content: Some(cm::MessageContent::Text("秋".into())),
                status: Some(cm::NodeStatus::Streaming),
                ..Default::default()
            })
            .await;
        env.sink()
            .emit(cm::ChatNode {
                id: "n-1".into(),
                delta: Some("天好".into()),
                ..Default::default()
            })
            .await;
        Ok(TurnOutput {
            text: "秋天好".into(),
            response_text_child_id: "n-1".into(),
            ..Default::default()
        })
    }
}

/// loop 侧轮次执行的夹具：内存事实轨 + 假会话动作面。
///
/// 事实轨是 core 的 [`EventStore`]（值面），不是会话存储实现——轮内写方经
/// `Store` 契约落格，与生产同一 trait；WAL 重开 / 归档语义由 session 的存储层
/// 用例覆盖（D-084 §1-A：事实以值交给执行层）。
struct Fixture {
    session: Arc<LoopSession>,
    facts: Arc<EventStore>,
    port: Arc<FakeSessionPort>,
}

fn setup_full() -> Fixture {
    setup_full_with(false)
}

/// 同 [`setup_full`]，另可开技能编译——技能编译的**编译**半边要它才动
/// （观测半边不需要，`memory.recalled` 与开关无关）。
fn setup_full_with(skill_compile: bool) -> Fixture {
    setup_with_settings(move |settings| settings.skill_compile_enabled = skill_compile)
}

/// 汇报那条产线的夹具：阈值配到「第一个批次边界就到期」。
///
/// `interval = 0` 让静默时长恒成立、`min_rounds = 0` 让轮数门槛只剩「> 0」
/// （即至少跑完一批工具）——本夹具要验的是**派发点接回了行表**，不是时长分档。
fn setup_with_reports() -> Fixture {
    setup_with_settings(|settings| {
        settings.progress_enabled = true;
        settings.progress_interval_ms = 0;
        settings.progress_min_rounds = 0;
        settings.progress_max_per_turn = 2;
    })
}

/// 同 [`setup_full`]，另可按需改本轮配置快照。
fn setup_with_settings(tune: impl FnOnce(&mut symbio_core::TurnSettings)) -> Fixture {
    let facts = Arc::new(EventStore::new());
    let port = Arc::new(FakeSessionPort::new(Vec::new()).with_facts(facts.clone()));
    let mut settings = default_settings();
    tune(&mut settings);
    let spec = turn_spec(
        "turn_exec_test",
        std::env::temp_dir().join("symbio-test/tool-archives"),
        std::env::temp_dir().join("symbio-test/transcripts"),
        settings,
    );
    let session = Arc::new(loop_session(spec, facts.clone(), port.clone()));
    Fixture {
        session,
        facts,
        port,
    }
}

// ==================== 一幕的直线（生产入口 `DriveStart::Control`）====================
//
// 三条判据各钉一环幕落的一格：备输入的出口（`result:"ready"`）、重试分支的登记、
// 定稿答话轮就地收束（`result:"closed"`）。入口取**生产那一个**：控制面交一幕，
// 环自己开幕、选债、收幕——夹具只备请求面，不替环跑任何一行。

/// 控制面夹具：请求面（编排器 / 上下文 / 本轮状态）+ 待追加的那条用户消息。
struct ControlHarness {
    facts: Arc<EventStore>,
    sink: symbio_core::ExecEventSink,
    ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    orchestrator: Arc<super::super::chat_loop::state::ChatOrchestrator>,
    context: super::super::chat_loop::state::SessionContext,
    turn: super::super::chat_loop::state::TurnState,
    req: super::super::chat_loop::state::TurnRequest,
    message: Option<cm::ChatNode>,
}

/// 判决 / 措辞 / 压缩**都关掉**：本组问的是幕的骨架，不是判决判成什么。
fn control_harness() -> ControlHarness {
    let facts = Arc::new(EventStore::new());
    let port = Arc::new(FakeSessionPort::new(Vec::new()).with_facts(facts.clone()));
    let mut settings = default_settings();
    settings.classify_enabled = false;
    settings.compose_enabled = false;
    settings.auto_compress = false;
    let spec = turn_spec(
        "turn_exec_control",
        std::env::temp_dir().join("symbio-test/tool-archives"),
        std::env::temp_dir().join("symbio-test/transcripts"),
        settings,
    );
    let session = Arc::new(loop_session(spec, facts.clone(), port));
    let sink =
        symbio_core::ExecEventSink::direct(Arc::new(CollectingFrames(Mutex::new(Vec::new()))));
    let orchestrator = Arc::new(super::super::chat_loop::state::ChatOrchestrator {
        provider: Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        parent: None,
        context_limit: 8192,
        session: session.clone(),
        writer: Arc::new(DiscardWriter),
        compose_enabled: false,
        opening_enabled: false,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    });
    ControlHarness {
        facts,
        sink,
        ctx: test_ctx(),
        orchestrator,
        context: super::super::chat_loop::state::SessionContext {
            messages: Vec::new(),
            session,
            consolidated: None,
        },
        turn: super::super::chat_loop::state::TurnState {
            abort: symbio_core::ExecAbortSignal::new(),
            ..Default::default()
        },
        req: super::super::chat_loop::state::TurnRequest {
            auto_compress: false,
            load_history: true,
            system_prompt: None,
        },
        message: Some(cm::ChatNode {
            id: "m1".into(),
            principal: Some(symbio_core::principal::Principal::new(
                symbio_core::principal::PrincipalKind::Person,
                "user",
            )),
            msg_type: Some(cm::NodeKind::Body),
            content: Some(cm::MessageContent::Text("你好".into())),
            ..Default::default()
        }),
    }
}

/// 走生产入口跑这一幕（开幕 / 选债 / 收幕都在环里）。
async fn drive_control(h: &mut ControlHarness) -> Result<super::DriveEnd, super::TurnExecExit> {
    super::drive(
        super::DriveStart::Control(Box::new(super::Control {
            orchestrator: &h.orchestrator,
            orchestrator_arc: &h.orchestrator,
            ctx: &h.ctx,
            sink: &h.sink,
            context: &mut h.context,
            turn: &mut h.turn,
            req: &h.req,
            single_message: h.message.take(),
            resume_kind: None,
        })),
        super::super::loop_rows::ACTORS,
    )
    .await
}

/// 本幕的 `inputs` 阶段格。
fn inputs_fact(facts: &[Event]) -> Option<&Event> {
    facts.iter().find(|e| {
        e.kind == "turn.stage" && e.payload.get("stage").and_then(|v| v.as_str()) == Some("inputs")
    })
}

/// 备输入落的是一格 `inputs` 阶段事实（`result:"ready"`），随后这一幕照常跑完。
#[tokio::test]
async fn drive_lands_the_inputs_stage_and_runs_the_turn() {
    use symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;

    let mut h = control_harness();
    let end = drive_control(&mut h).await.expect("幕跑得完");
    assert!(
        matches!(end, super::DriveEnd::Completed),
        "正常一幕收工（控制面交回已收束）"
    );

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
async fn drive_lands_one_branch_fact_for_a_retry_and_consumes_the_registration() {
    use symbio_plugin_api::events::EVENT_TURN_BRANCH;

    let mut h = control_harness();
    h.turn.retry_of = Some("t-old".to_string());
    let end = drive_control(&mut h).await.expect("幕跑得完");
    assert!(matches!(end, super::DriveEnd::Completed));

    assert!(
        h.turn.retry_of.is_none(),
        "寄存的旧根被取走：同一幕不会落两条分支格"
    );
    let root_id = h
        .turn
        .branch_root()
        .expect("收尾能从事实切片读回本幕的根（组内标注要用）");

    let branch_facts = |h: &ControlHarness| -> Vec<Event> {
        h.facts
            .range(Seq::new(0))
            .into_iter()
            .filter(|e| e.kind == EVENT_TURN_BRANCH)
            .collect()
    };
    let landed = branch_facts(&h);
    assert_eq!(landed.len(), 1, "一次重试只开一支：{landed:?}");
    assert_eq!(landed[0].payload["of"], "t-old");
    assert_eq!(landed[0].payload["into"], root_id.as_str());
    assert_eq!(
        landed[0].produced_by,
        Some(h.turn.user_seq),
        "分支格是断言类：溯源必须指向本轮用户格"
    );
    assert!(!landed[0].actor.is_empty(), "actor 取本轮主体，不留空");

    // 续写幕：`retry_of` 已被取走 ⇒ 再走一次登记是空操作，事实面不多第二支。
    // 不真跑第二幕：同一轮已经收束，运行器不会再收束第二条；这里直接问那个登记口
    // ——它正是每一幕都会调的那一处。
    h.turn.drive_seq += 100;
    let store: Arc<dyn Store<Event = Event>> = h.facts.clone();
    super::land_retry_branch(store.as_ref(), h.ctx.as_ref(), &mut h.turn, "r-next")
        .expect("登记是空操作");
    assert_eq!(branch_facts(&h).len(), 1, "幕间续写不再开第二支");
}

/// 定稿答话轮：轮首就地收束 ⇒ 环**不进**轮内环，一次模型往返都不发。
///
/// 收束格（`inputs.result:"closed"`）由编排层在 `DriveEnd::Answered` 之后落——环
/// 里没有编排器。本用例照 `chat_loop` 那一臂问同一个落格口：只落 `ready` 的话，
/// 收束的那一幕在事实面上与「正常跑完」同形——一个已经答完的轮会再答一次。
#[tokio::test]
async fn drive_stops_at_an_in_place_answer_and_lands_closed_without_a_model_request() {
    let mut h = control_harness();
    // 判决执行体寄存下来的那体答话（`run_classify` 的产出形态）。
    h.turn.head_answer = Some(super::super::chat_loop::AnsweredTurn {
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

    let end = drive_control(&mut h).await.expect("收束不是失败");
    assert!(
        matches!(end, super::DriveEnd::Answered),
        "判成答话的一轮交回 `Answered`，不进轮内环"
    );

    let store: Arc<dyn Store<Event = Event>> = h.facts.clone();
    super::land_inputs(store.as_ref(), h.ctx.as_ref(), &h.turn, "closed").expect("收束也要落格");

    let all = h.facts.range(Seq::new(0));
    let landed = inputs_fact(&all).expect("收束也要落格（否则与「还没备」同形）");
    assert_eq!(landed.payload["result"], "closed");
    assert!(
        all.iter()
            .filter(|e| e.kind == "request" && e.payload["port"] == "model")
            .count()
            == 0,
        "定稿答话轮不进轮内环，零模型往返"
    );
}

/// 成功轮：事实原生入格（user + final，不变量绿），增量经 UiBridge 回 UI，
/// 全文与分片拼接一致、正文节点 id 稳定。
#[tokio::test]
async fn tool_free_turn_writes_grid_and_streams_ui() {
    let fx = setup_full();
    // 收集口：ExecEventSink::direct 的 writer 不可回读，自持一份 Arc 引用帧。
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    let res = drive_turn(tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    ))
    .await
    .expect("v2 执行成功");
    let out = res.output;
    assert!(
        res.messages.is_empty(),
        "无工具轮不额外产生消息（助手轮消息由步骤 5 的 into_messages 落）：{:?}",
        res.messages
    );

    // 聚合全文 + 收口用的正文节点 id（步骤 5-7 靠它定格子节点）。节点 id 由
    // **v2 侧分配**（UiBridge 首片）——模型插件的内部节点 id 是插件内务，
    // 不过桥；两处 id 必须同一个，否则终态帧会落在不存在的节点上。
    assert_eq!(out.text, "秋天好");
    assert!(
        !out.response_text_child_id.is_empty(),
        "正文节点 id 必须已分配"
    );

    // UI 帧：接缝先发 Turn 根骨架（新幕 `fresh_root`），其后是首片快照建节点
    // （Streaming）与窄帧追加——与 model 插件直发同构。
    let got = frames.0.lock().unwrap();
    assert_eq!(got.len(), 3, "根骨架 + 快照 + 窄帧，不多不少：{got:?}");
    assert_eq!(got[0].id, "r-1", "新幕的根节点先上线");
    assert_eq!(got[0].msg_type, Some(cm::NodeKind::Skeleton));
    assert_eq!(got[0].status, Some(cm::NodeStatus::Streaming));
    assert_eq!(
        got[1].id, out.response_text_child_id,
        "收口用的 id 就是首片建的节点"
    );
    assert_eq!(got[1].status, Some(cm::NodeStatus::Streaming));
    assert_eq!(got[1].parent_id.as_deref(), Some("r-1"));
    assert!(
        matches!(&got[1].content, Some(cm::MessageContent::Text(t)) if t == "秋"),
        "快照帧带首片全文：{:?}",
        got[1].content
    );
    assert_eq!(got[2].id, out.response_text_child_id, "窄帧同一节点");
    assert_eq!(got[2].delta.as_deref(), Some("天好"), "窄帧只带增量");

    // 事实网格：user 格 + final 格，不变量全绿，final 溯源指向本轮用户格。
    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + final 格：{snap:?}");
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(snap[0].turn, 0, "turn 号 = WAL 内 user.message 计数");
    assert_eq!(snap[1].kind, EVENT_CHAT_ASSISTANT_FINAL);
    assert_eq!(snap[1].payload["text"], "", "final 只存结局");
    let output = all
        .iter()
        .rev()
        .find(|e| e.kind == "response" && e.payload["port"] == "model")
        .expect("模型应答在");
    assert_eq!(
        output.payload["answer"], "秋天好",
        "聚合全文归那一次模型往返"
    );
    assert!(snap[1].produced_by.is_some(), "final 溯源指向用户格");
}

/// 失败轮（I3）：fallback 格照落（网格不缺格），错误上抛给 chat_loop 走
/// Failed 出口。
#[tokio::test]
async fn tool_free_turn_failure_leaves_fallback_event_and_bubbles() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    let err = drive_turn(tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Fail).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    ))
    .await
    .err()
    .expect("失败必须上抛（Failed 出口）");

    assert!(
        matches!(
            err,
            TurnExecExit::Failed(PluginError::InternalError(ref m)) if m.contains("模型断了")
        ),
        "上抛的是生成失败本体：{err:?}"
    );

    // 网格里用户格 + fallback 格都在——失败也是一句话，不是静默。
    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + fallback 格：{snap:?}");
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
    assert_eq!(snap[1].kind, EVENT_CHAT_ASSISTANT_FALLBACK);
    let why = snap[1].payload["detail"].as_str().unwrap_or_default();
    assert!(why.contains("模型断了"), "兜底格要记失败原因：{why}");
    assert_eq!(
        snap[1].payload["reason"], "fallback",
        "停机原因是闭集里的取值，不是自由文本"
    );
}

/// 中止轮：**照落收束格**（`chat.assistant.final`，载荷 `status: "aborted"` +
/// `terminated`），上抛 `Aborted` 走独立出口——用户自己按的停止不该有错误条与
/// 重试入口，也不进兜底分子；本轮无半截产出时 `partial` 为空。
#[tokio::test]
async fn tool_free_turn_abort_closes_the_cell_and_bubbles_aborted() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    let err = drive_turn(tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Abort).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    ))
    .await
    .err()
    .expect("中止必须上抛（Aborted 出口，不是 Failed）");

    let TurnExecExit::Aborted(partial) = &err else {
        panic!("上抛的是中止本体，不被压成失败：{err:?}");
    };
    assert!(partial.is_empty(), "本轮没有半截产出：{partial:?}");

    // 网格：用户格 + 收束格——中止照落收束格（kind 不换），绝不落兜底格。
    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + 收束格：{snap:?}");
    assert_eq!(snap[1].kind, EVENT_CHAT_ASSISTANT_FINAL, "中止照落收束格");
    assert_eq!(snap[1].payload["status"], "aborted", "终止是状态字段");
    assert_eq!(
        snap[1].payload["reason"], "interrupted",
        "停机原因入格（顶层 `reason`，闭集取值）"
    );
    assert!(
        snap[1].payload["terminated"]["at"].as_i64().is_some(),
        "终止时刻入格"
    );
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
}

/// 事实完整性（判据）：**中止后那半句话还在**——模型说到一半（"让我想想
/// 这个名词"）被打断，半截正文 / 半截思考（一）随出口带走
/// （`TurnExecExit::Aborted(partial)`，由消费循环 `persist_failure` 收尾入格），
/// （二）照原样入格
/// （收束格载荷 `text` / `reasoning`）。用户随后追问那个名词时，前置上下文
/// （那半句 + 半截思考）在重建的历史里都在。
#[tokio::test]
async fn aborted_half_sentence_survives_in_partial_and_grid() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    let err = drive_turn(tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::AbortAfterPartial).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    ))
    .await
    .err()
    .expect("中止必须上抛（Aborted 出口）");

    // ① 随出口带走：正文节点 + 推理节点都进 `partial`（落库镜像的来源）。
    let TurnExecExit::Aborted(partial) = &err else {
        panic!("上抛的是中止本体：{err:?}");
    };
    let texts: Vec<String> = partial
        .iter()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_text()))
        .collect();
    assert_eq!(
        texts,
        ["让我想想这个名词", "这个名词需要先定义。"],
        "半截正文与半截思考都要在（正文节点在前）：{partial:?}"
    );
    assert!(
        partial.iter().any(|m| m.is_reasoning()),
        "推理单独成节点（不是拼进正文）：{partial:?}"
    );

    // ② 入格：半截正文 / 思考归那一次模型往返（`failed`），收束格只引用结局。
    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + 收束格：{snap:?}");
    let cell = snap[1];
    assert_eq!(cell.kind, EVENT_CHAT_ASSISTANT_FINAL);
    assert_eq!(cell.payload["status"], "aborted");
    // 停机原因在**顶层** `reason`（闭集），`terminated` 只留时刻。
    assert_eq!(cell.payload["reason"], "interrupted");
    assert!(
        cell.payload["terminated"]["at"].as_i64().is_some(),
        "半截产出停在哪里由 `terminated.at` 说，原因不再在 terminated 里写第二份"
    );
    let output = all
        .iter()
        .find(|e| e.kind == "failed" && e.payload["port"] == "model")
        .expect("模型失败格在");
    assert_eq!(output.payload["text"], "让我想想这个名词", "半截正文入格");
    assert_eq!(
        output.payload["reasoning"], "这个名词需要先定义。",
        "半截思考入格（reasoning 在网格里查得到）"
    );
    assert_eq!(cell.payload["answer_id"], output.payload["answer_id"]);
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
}

/// 工具轮：一次工具调用 ⇒ 产物落格 + 中途正文定格 + 工具节点成形 +
/// **工具结果进了下一次请求**。
///
/// 这条用例的判据刻意分成三处，各自只能证明一件事：
/// - 网格（`artifact.added` + 溯源）：事实真的入了格；
/// - UI 帧（定格 + 工具节点 + 结果节点）：用户真的看得见这一轮；
/// - **回读下一次请求**：结果真的回了模型——网格与帧都证明不了这条
///   （结果可以既入格又入帧，却漏在 prompt 之外）。
#[tokio::test]
async fn tool_round_lands_artifact_and_feeds_next_request() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::ToolThenAnswer);

    let user_seq = seed_user(&fx, 0, "读一下 a.md");
    let res = drive_turn(ExecTurn {
        turn_no: 0,
        user_seq,
        drive_seq: user_seq,
        session: &fx.session,
        provider: Arc::new(provider),
        // 无插件宿主 ⇒ 工具分发按「没有父插件」诚实失败——本用例验的是**链路**
        // （落格 / 帧 / 回灌），工具真正跑起来由 e2e 覆盖。失败是信息性的：
        // 结果照样回灌，循环照常继续。
        parent: None,
        ctx: test_ctx(),
        system_prompt: "system",
        abort: symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            category: None,
            examples: None,
            ..Default::default()
        }],
        recalled: None,
        reflex_text: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        report: None,
        // 开场白：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    })
    .await
    .expect("工具轮执行成功");
    let out = res.output;
    assert_eq!(out.text, "秋天好", "收尾轮的正文");

    // ── 回读请求体：工具结果进了下一次请求 ────────────────────────────────
    let seen = prompts.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "工具轮 + 收尾轮 = 两次请求：{seen:?}");
    let joined = |i: usize| -> String {
        seen[i]
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let has_role = |i: usize, role: &str, needle: &str| -> bool {
        seen[i].iter().any(|(r, t)| r == role && t.contains(needle))
    };
    assert!(
        joined(0).contains("读一下 a.md"),
        "第一次请求带本轮用户发言：{}",
        joined(0)
    );
    assert!(
        !seen[0].iter().any(|(r, _)| r == "tool"),
        "第一次请求还没有 role=tool 消息可言：{:?}",
        seen[0]
    );
    assert!(
        joined(1).contains("vdfs_read"),
        "第二次请求要带上模型请求过的工具名：{}",
        joined(1)
    );
    assert!(
        joined(1).contains("No parent plugin"),
        "第二次请求要带上工具结果正文（含失败——失败也是信息）：{}",
        joined(1)
    );
    // D-048a 的核心断言：工具结果与助手发言**各带各的角色**（只取 `messages.first`
    // 的单条形态测不出它——整段挤在一条 user 消息里，`role == "tool"` 也不会命中）。
    assert!(
        has_role(1, "tool", "No parent plugin"),
        "第二次请求的工具结果须是 role=tool 消息：{:?}",
        seen[1]
    );
    assert!(
        has_role(1, "assistant", "我先查一下"),
        "助手的工具轮发言须是 role=assistant：{:?}",
        seen[1]
    );

    // ── 落库消息：助手中途正文 + ToolCall + 工具结果 ──────────────────────
    assert_eq!(
        res.messages.len(),
        3,
        "正文 + 工具调用 + 工具结果：{:?}",
        res.messages
    );
    assert_eq!(res.messages[0].msg_type, Some(cm::NodeKind::Body));
    assert_eq!(res.messages[0].status, Some(cm::NodeStatus::Completed));
    assert_eq!(res.messages[0].parent_id.as_deref(), Some("r-1"));
    let call = &res.messages[1];
    assert_eq!(call.msg_type, Some(cm::NodeKind::Skeleton));
    assert_eq!(call.style_tag(), Some(cm::CHAT_STYLE_TAG_TOOL));
    assert_eq!(call.id, "tc-1", "工具节点 id 取 provider 给的节点 id");
    assert_eq!(
        call.tool_call_id.as_deref(),
        Some("call_1"),
        "wire id 原样保留"
    );
    let tool_result = &res.messages[2];
    assert_eq!(tool_result.parent_id.as_deref(), Some("tc-1"));
    assert_eq!(tool_result.node_role(), Some(cm::NodeRole::Tool));

    // ── UI 帧：中途正文被**定格**（不是永远流式中），工具节点与结果都在 ────
    let got = frames.0.lock().unwrap();
    let text_frames: Vec<_> = got
        .iter()
        .filter(|m| m.is_text() && m.node_role() == Some(cm::NodeRole::Assistant))
        .collect();
    assert!(
        text_frames
            .iter()
            .any(|m| m.status == Some(cm::NodeStatus::Completed)),
        "中途正文必须被定格：{got:?}"
    );
    assert!(
        got.iter().any(|m| m.is_tool_call() && m.id == "tc-1"),
        "工具调用节点必须广播给前端：{got:?}"
    );
    assert!(
        got.iter()
            .any(|m| m.node_role() == Some(cm::NodeRole::Tool)
                && matches!(&m.content, Some(cm::MessageContent::Text(t)) if t.contains("No parent plugin"))),
        "工具结果节点必须广播给前端：{got:?}"
    );

    // ── 网格：用户格 + 产物格 + final 格，产物格溯源指向本轮用户格 ────────
    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 3, "用户格 + 产物格 + final 格：{snap:?}");
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(
        snap[1].kind,
        symbio_plugin_api::events::EVENT_ARTIFACT_ADDED
    );
    assert_eq!(snap[1].entity, Entity::Artifact);
    assert_eq!(snap[1].verb, Verb::Asserted);
    assert_eq!(snap[1].payload["tool"], "vdfs_read");
    assert!(
        snap[1].payload["text"]
            .as_str()
            .unwrap_or_default()
            .contains("No parent plugin"),
        "产物格落的是结果正文：{:?}",
        snap[1].payload
    );
    assert_eq!(
        snap[1].produced_by,
        Some(0),
        "产物格溯源指向本轮用户格（DE-032 §3 的 produced_by 断言）"
    );
    assert_eq!(snap[2].kind, EVENT_CHAT_ASSISTANT_FINAL);
}

/// 轮边界汇报「产生即上线」（「轮中对话面节点不入实时面」的另一半）：
/// `chat.assistant.reported` 落格的那一刻，同一个节点就经流式桥到实时面，
/// 而不是等轮末的落库镜像一起到。
///
/// ## 三段各证明什么
///
/// - **时序**：汇报帧在收尾正文帧**之前**到达——轮末才到就是这条缺陷本体
///   （一次几分钟的工具轮全程无反馈）。
/// - **身份一致**：WAL 载荷的 `id`、实时面那一格、落库镜像那一格的 id **是同一个**。
///   重建侧 `reported_node` 读的就是载荷里那个 `id`；id 一变，同一句汇报在
///   实时面与重建面就是两张卡。
/// - **不挂死**：汇报闭包里多一份 `Arc<UiBridge>`，而函数末尾那次 `drop(bridge)`
///   是发射端退出**唯一**的信号。那份克隆若活得比重构的收束点还久，本用例不是
///   断言红而是整个挂住——所以这条时序必须由用例钉住，不能靠读码推理。
///
/// ## 反向自检（两条都实跑过）
///
/// 去掉 `live.on_tool_frame(&node)` ⇒ ① 红（帧根本没到，②随之无从谈起）；
/// 把载荷里那个 `id` 字段去掉 ⇒ 「载荷要带节点 id」那条红（重建侧只能退回事件 id，
/// 与实时面就是两个身份）。
#[tokio::test]
async fn round_report_reaches_the_live_surface_before_turn_end() {
    // 汇报这条产线要 **行表真的开账**（`policy.report` 欠不欠由 `report_due`
    // 判），所以夹具把三个阈值配到「第一个批次边界就到期」，不靠手造一个「该报了」
    // 的返回值——那正是退役的 `RoundHookFn` 掊着的判定。
    let fx = setup_with_reports();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::ToolThenAnswer);

    // 措辞能力口：只产一句话（判定与落格都不在这里，D-163）。配额 2 让两个批次
    // 边界都能报，但本用例的编排只有一批工具，故只报一次。
    let text: super::super::report_exec::ReportTextFn = Arc::new(|_snapshot| {
        Box::pin(async { Some("已经完成 1 轮工具调用，还在继续。".to_string()) })
    });

    let user_seq = seed_user(&fx, 0, "读一下 a.md");
    seed_config(&fx, 0, user_seq);
    // 本幕的「现在」不再由用例递给派发点：环读事实折出静默时长，时刻与它同源
    // （见 `drive_turn` 里那次 `wake_now_ms`）。间隔配成 0，到期判定与它无关。
    let res = drive_turn(ExecTurn {
        turn_no: 0,
        user_seq,
        drive_seq: user_seq,
        session: &fx.session,
        provider: Arc::new(provider),
        parent: None,
        ctx: test_ctx(),
        system_prompt: "system",
        abort: symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            category: None,
            examples: None,
            ..Default::default()
        }],
        recalled: None,
        reflex_text: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        report: Some(super::super::report_exec::ReportPort { text }),
        opening_text: None,
        opening_dialog: None,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    })
    .await
    .expect("带汇报的工具轮执行成功");

    let said = |m: &cm::ChatNode, needle: &str| matches!(&m.content, Some(cm::MessageContent::Text(t)) if t.contains(needle));

    // ── ① 时序：汇报在收尾正文之前就到实时面 ─────────────────────────────
    let got = frames.0.lock().unwrap();
    let report_at = got.iter().position(|m| said(m, "还在继续")).expect(
        "汇报必须在产生的那一刻就进实时面：等轮末落库镜像才到，就是「长轮次全程无反馈」本体",
    );
    // 收尾正文在实时面上是「快照 + 窄增量」两段（先「秋」再「天好」），所以
    // 「这句说到第几帧才算说全」得沿帧序累积着看——只比快照会把答案整个漏掉。
    let answer_at = {
        let mut acc = String::new();
        got.iter().enumerate().find_map(|(i, m)| {
            let piece = match &m.content {
                Some(cm::MessageContent::Text(s)) => s.as_str(),
                _ => m.delta.as_deref().unwrap_or_default(),
            };
            acc.push_str(piece);
            acc.contains("秋天好").then_some(i)
        })
    }
    .expect("收尾正文也在实时面上");
    assert!(
        report_at < answer_at,
        "汇报要比收尾轮的答案先到（它是**轮中**说的一句话）：report={report_at} answer={answer_at}"
    );

    // ── ② 身份：载荷 / 实时面 / 落库镜像三处同一个 id ────────────────────
    let all = fx.facts.range(Seq::new(0));
    let reported = all
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED)
        .unwrap_or_else(|| panic!("汇报照常落格：{all:?}"));
    let payload_id = reported.payload["id"].as_str().unwrap_or_default();
    assert!(
        !payload_id.is_empty(),
        "载荷要带节点 id，重建侧 `event_payload_node_id` 才读得到：{:?}",
        reported.payload
    );
    assert_eq!(
        reported.produced_by,
        Some(0),
        "断言类事件带溯源（I2），指向本轮用户格"
    );
    let frame = got
        .iter()
        .find(|m| said(m, "还在继续"))
        .expect("① 已经证明它在");
    assert_eq!(
        frame.id, payload_id,
        "实时面那一格与事实载荷同一个 id——否则轮末重建会多出一张卡"
    );
    let mirrored = res
        .messages
        .iter()
        .find(|m| said(m, "还在继续"))
        .expect("汇报同样进落库镜像（前端读它）");
    assert_eq!(
        mirrored.id, payload_id,
        "落库镜像与实时面同一个身份：{mirrored:?} vs {frame:?}"
    );
    assert_eq!(
        all.iter()
            .filter(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED)
            .count(),
        1,
        "只报一次（游标从 0 起 ⇒ 事件 id `rp-{{turn}}-0`）：{all:?}"
    );
}

// ==================== 收束派生事实：记忆（检索 / 巩固；编码不归本插件）====================

/// **收束派生事实**：检索锚与巩固必须由 `derived_facts::record_learning` 落格；
/// 编码、技能观测与编译**不得**由本插件写。
///
/// ## 这个用例挡的是什么
///
/// 轮次事实（用户格 / final 格 / 产物格）由 v2 运行器原生记账，而记忆**不是轮次
/// 事实**——运行器一处都不写。编码自 D-263 起是**情景记忆能力包的收束贡献**
/// （`crates/symbio-plugin-memory_episodic`）：本插件一侧再写一笔就是同一事实两个
/// 写方。所以这里反向钉住「一轮跑完，本插件不落任何 `memory.encoded`」——正向判据
/// （带溯源入格、同文去重、截断、可见域）随写方迁进那个包的 `contributor.test.rs`，
/// 端到端由 e2e `t29` 带。
///
/// 技能观测与编译在 D-262 之后同理是**技能能力包的收束贡献**（`Contributor`）：
/// 注入一份"看起来命中了技能"的夹具之后，`memory.calibrated` 与
/// `memory.encoded{tag:"skill"}` 都不得出现。
///
/// **反向自检**：把 `record_learning` 的巩固写口改回自己编码（在 loop 里落
/// `memory.encoded`），本用例必须红。
#[tokio::test]
async fn full_turn_writes_no_memory_encoding_from_the_host() {
    let fx = setup_full_with(true);
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::Faithful);

    let req = tool_free_req(
        &fx,
        Arc::new(provider),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    );
    drive_turn(req).await.expect("full 轮执行成功");

    let events = fx.facts.range(Seq::new(0));
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();

    let encoded: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == crate::control_decl::EVENT_MEMORY_ENCODED)
        .collect();
    assert!(
        encoded.is_empty(),
        "编码不归本插件写（D-263：情景记忆能力包的收束贡献）：{encoded:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind == symbio_plugin_api::events::EVENT_MEMORY_CALIBRATED),
        "技能观测不归本插件写（D-262：交接缝贡献）：{kinds:?}"
    );
}

// ==================== 收束派生事实：承诺 / 任务表 / 熔断 ====================

/// **收束派生事实**：承诺（`agent_run` 的代际立约）必须入格。
///
/// ## 这个用例挡的是什么
///
/// 承诺的数据来源在工具执行层（`EffectInput`，`process_tool_calls_async` 的出参），
/// 写方是 `derived_facts::record_derived`（`turn_exec` 轮末调）。若分发方不把出参交回
/// （`SessionDispatchPort` 的三个出参收在本地 `Vec`、经 `take_derived` 交回，
/// 不收不交就等于丢），承诺就**静默全丢**——代际立约整体失效。
///
/// 本用例走的是**真实通道**：假 provider 回一个 `agent_run` 工具调用 ⇒
/// `SessionDispatchPort::dispatch` 收集出参 ⇒ `drive_turn` 轮末调 `record_derived`
/// 落格。它同时钉住两件事——出参通道（`take_derived`）与写方（`record_derived`）——
/// 以及锚点：立约溯源必须是本轮开口（I2）。
///
/// **反向自检**：把 `tool_dispatch` 里三份出参改回 `&mut Vec::new()`，本用例必须红。
#[tokio::test]
async fn full_turn_lands_derived_commitment_facts() {
    use symbio_plugin_api::events::{EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OPENED};

    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::ToolThenAgentRun);

    // 注册面与模型面同一份元数据：`agent_run` 自声明「派活 + 立约」两个行为名，
    // 承诺记账只认声明——与生产（`crates/symbio-plugin-agent/src/host/subagent.rs`）逐字同形。
    let tool_meta = CapabilityMeta {
        name: "agent_run".into(),
        description: "派生一个子智能体".into(),
        input_schema: serde_json::json!({ "type": "object" }),
        category: None,
        examples: None,
        behaviors: &[
            symbio_plugin_api::behaviors::ASSIGN_WORK,
            symbio_plugin_api::behaviors::DELEGATION_PROMISE,
        ],
        ..Default::default()
    };

    let user_seq = seed_user(&fx, 0, "请委托评审");
    let res = drive_turn(ExecTurn {
        turn_no: 0,
        user_seq,
        drive_seq: user_seq,
        session: &fx.session,
        provider: Arc::new(provider),
        // 无插件宿主 ⇒ `agent_run` 诚实失败（"No parent plugin"）。失败同样是
        // **没履约**——立约与违约都要入格（DE-038 §5：违约可被观测是 T5 的前提）。
        parent: None,
        ctx: test_ctx_declaring(std::slice::from_ref(&tool_meta)).await,
        system_prompt: "system",
        abort: symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "请委托评审",
        window_turns: 6,
        tools: std::slice::from_ref(&tool_meta),
        recalled: None,
        reflex_text: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        report: None,
        // 开场白：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: symbio_core::node_build::PresentationTree::Grouped,
    })
    .await
    .expect("工具轮执行成功");
    assert_eq!(res.output.text, "秋天好", "收尾轮的正文");

    let events = fx.facts.range(Seq::new(0));
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();

    let user_seq = events
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.seq.map(|s| s.value()))
        .expect("本轮用户格必须入格");

    let offered = events
        .iter()
        .find(|e| e.kind == EVENT_COMMITMENT_OPENED)
        .unwrap_or_else(|| panic!("代际立约必须入格（出参通道）：{kinds:?}"));
    assert_eq!(
        offered.payload["promise"], "让 reviewer 复查这段",
        "承诺内容 = 委托出去的那句话（`agent_run` 的 prompt）"
    );
    assert_eq!(
        offered.produced_by,
        Some(user_seq),
        "立约锚必须在本轮开口（I2：承诺溯源 100%）"
    );
    assert!(
        events.iter().any(|e| e.kind == EVENT_COMMITMENT_BROKEN),
        "无宿主 ⇒ 委托失败 ⇒ 违约必须可观测（DE-038 §5）：{kinds:?}"
    );
    assert!(
        check_all(guard_vocab(), &events).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &events)
    );
}

// ==================== 快路的收束半边 ====================

/// 轮首 host 已选中快路（出口格落了）⇒ 本轮**一次模型都不调**，以技能正文收束；
/// 派发格声明 `reflex`，收束格 `cost_ms` 在反射档预算内。
///
/// ## 判据只有一个，就是"没调模型"
///
/// `prompts` 为空是本用例最硬的一条。技能命中若只是"把技能正文当提示词喂给模型"，
/// 请求数照样是 1——那样"跳过模型"就是一句空话，DE-041 §6.2 的加速根本不存在。
/// 其余三条（`tier = reflex` / `model = reflex` / `cost_ms` 在预算内）钉的是**埋点**：
/// 没有它们，"跳过"发生了却无从观测。
///
/// ## 选中的那一半在别处
///
/// 判定（逐字命中 ∧ 只对新开轮）与作答都是 `skill.recall` 那一行的事（D-262）；
/// 宿主只代落 `stage:"reflex"` 并经 `set_reflex` 把正文寄进本轮状态（D-260）。
/// 本用例从**这两样产物**出发，钉的是收束：`turn_head.reflex_answered` 让
/// `close-turn` 成为第一笔债，`announce-model` 不欠 ⇒ 模型一次不被调。
#[tokio::test]
async fn a_reflex_answer_closes_without_any_model_call() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::Faithful);

    // `trigger` 与 `tool_free_req` 的 `user_text`（"你好"）逐字相同 ⇒ 命中。
    let mut req = tool_free_req(
        &fx,
        Arc::new(provider),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        "你好",
    );
    // 生产侧由宿主在 `RowStep::Answer` 之后落这一格（D-260）。
    super::super::stage_events::land(
        fx.facts.as_ref(),
        0,
        req.user_seq,
        "system",
        &[super::super::stage_events::StageFact::reflex(
            0,
            req.user_seq,
            "sk-1",
        )],
    )
    .expect("快路出口格落格");
    // 正文由宿主在同一处寄进草稿（能力只答，不碰草稿）。
    req.reflex_text = Some("先列要点；再合并同类项。");
    let res = drive_turn(req).await.expect("快路执行成功");

    assert_eq!(
        res.output.text, "先列要点；再合并同类项。",
        "命中即以技能正文收束（不是让模型照着技能重说一遍）"
    );
    assert!(
        prompts.lock().unwrap().is_empty(),
        "命中技能 ⇒ 一次模型调用都不发生：{:?}",
        prompts.lock().unwrap()
    );
    assert!(
        !res.output.response_text_child_id.is_empty(),
        "产物必须经桥上线（否则收束节点在前端建不起来）"
    );
    assert!(
        res.messages.is_empty(),
        "反射档不跑工具 ⇒ 没有额外消息：{:?}",
        res.messages
    );

    let all = fx.facts.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "开轮格 + 收束格（没有产物格）：{snap:?}");
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    let dispatch = all
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_TURN_DISPATCH && e.turn == 0)
        .expect("反射档同样要有派发格");
    assert_eq!(
        dispatch.payload["tier"], "reflex",
        "派发格声明反射档（DE-041 §3 的档位观测；兜底率按层统计靠它）"
    );
    assert_eq!(
        dispatch.payload["budget_ms"],
        LatencyTier::Reflex.budget_ms(),
        "预算数值随派发格入格"
    );
    assert_eq!(snap[1].kind, EVENT_CHAT_ASSISTANT_FINAL);
    assert_eq!(snap[1].payload["text"], "先列要点；再合并同类项。");
    assert_eq!(
        snap[1].payload["model"], "reflex",
        "载荷的 `model` 记产者——空着会让命中轮与普通轮在事件面上无从区分"
    );
    assert!(
        snap[1].cost_ms <= LatencyTier::Reflex.budget_ms(),
        "命中后耗时必须在反射档预算内（DE-041 §6.2：命中显著低于未命中）：{}",
        snap[1].cost_ms
    );
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
}

/// 跨插件契约（D-262）：本轮的召回清单在事实面上带 `resumed` 一格，
/// 技能行据此判「唤醒幕不重答」（同一份清单不让它顶掉刚被批准的那次工具）。
///
/// 技能行自己不开判定账——判定随能力住在技能包；本插件
/// 只保证**它读得到那条判据**：`turn.recalled` 的每条目都带幕锚之后的召回阶段
/// 与 `resumed`。所以这里两侧各钉一遍：唤醒幕 ⇒ `resumed: true`；新开一轮 ⇒ `false`。
#[test]
fn the_recall_list_carries_the_resumed_flag() {
    // 事实轨以值注入（`EventStore`）：本用例不碰会话存储布局，只钉视图的判据。
    let store = EventStore::new();
    let user_seq = store
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "你好", "tier": "deep" })),
        )
        .expect("开轮格")
        .value();

    // 本幕锚 = 恢复阶段事实（`chat_loop::begin_drive` 在生产里落它）：它把幕锚从
    // 用户格挪到这一条 ⇒ `turn_head.resumed = true`。
    let resume_seq = super::super::stage_events::land_with_seqs(
        &store,
        0,
        user_seq,
        "system",
        &[super::super::stage_events::StageFact::resume(
            0, "approve", "1",
        )],
    )
    .expect("恢复阶段事实落格")
    .first()
    .copied()
    .expect("恢复阶段有 seq");

    let recalled = [(user_seq + 1, "skill".to_string())];
    super::super::stage_events::land(
        &store,
        0,
        resume_seq,
        "system",
        &[super::super::stage_events::StageFact::recall(
            0, resume_seq, &recalled, true,
        )],
    )
    .expect("召回阶段事实落格");

    let slice = store.range(Seq::new(0));
    let view = super::super::loop_rows::view("turn.recalled", &slice, 0, 0)
        .expect("turn.recalled 必须可求值");
    let entries = view.as_array().expect("视图出数组");
    assert_eq!(entries.len(), 1, "{view}");
    assert_eq!(entries[0]["recall"], recalled[0].0);
    assert_eq!(entries[0]["tag"], "skill");
    assert_eq!(
        entries[0]["resumed"], true,
        "唤醒幕的召回清单必须自报 resumed（技能行据此不重答）：{view}"
    );

    // 对照：同一形状换成新开一轮（锚 = 用户格）⇒ `resumed: false`——
    // 否则 `resumed` 恒真或恒假，上一条判据就恒过。
    let fresh = EventStore::new();
    let fresh_seq = fresh
        .append(
            Event::pending(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "你好", "tier": "deep" })),
        )
        .expect("开轮格")
        .value();
    super::super::stage_events::land(
        &fresh,
        0,
        fresh_seq,
        "system",
        &[super::super::stage_events::StageFact::recall(
            0, fresh_seq, &recalled, false,
        )],
    )
    .expect("召回阶段事实落格");
    let slice = fresh.range(Seq::new(0));
    let view = super::super::loop_rows::view("turn.recalled", &slice, 0, 0)
        .expect("turn.recalled 必须可求值");
    assert_eq!(
        view[0]["resumed"], false,
        "新开一轮的召回清单不得自称唤醒幕：{view}"
    );
}

// ── `UiBridge::flush` 的同步点 ────────────────────────────────────────────
//
// `flush` 是 `tool_dispatch::dispatch` 里**定格正文**与**工具节点**之间的同步点：定格帧
// 在桥的通道里排队，而工具节点直接写出口，不排空队列就倒序（工具卡片先于正文）。
//
// 第一条钉的是最容易被静默的那条出口——接收端**活着**但把屏障丢掉（emitter 被
// abort 或 panic）。原实现是 `let _ = wait.await`，吞掉的恰好是「同步点失效」这件事
// 本身：此后删掉屏障与屏障失效在日志里长得一模一样。判据取「**必须返回、不得挂死**」
// ——挂死意味着整轮停在正文定格处，比倒序更糟，也更难发现。
#[tokio::test]
async fn flush_returns_when_the_emitter_drops_the_barrier() {
    let q = Arc::new(FrameQueue::new(UI_FRAME_CAPACITY));
    // 接收端存活，收到屏障却直接丢弃（模拟 emitter 中途退出）
    let rx = q.clone();
    let _emitter = tokio::spawn(async move {
        while let Some(frame) = rx.pop().await {
            if let UiFrame::Barrier(done) = frame {
                drop(done);
                break;
            }
        }
    });
    let bridge = UiBridge {
        tx: q,
        root_id: "root".to_string(),
        node: Arc::new(Mutex::new(None)),
        reasoning: Arc::new(Mutex::new(None)),
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), bridge.flush())
        .await
        .expect("屏障被丢弃时 flush 必须返回，不得挂死");
}

// 第二条：接收端**已经没了**（连 send 都失败）。这条走的是早退分支，与上一条不是
// 同一条路径——收下它是因为 `flush` 被调用时生成往往已收尾，早退不能变成等待。
#[tokio::test]
async fn flush_returns_immediately_when_the_channel_is_already_closed() {
    let tx = Arc::new(FrameQueue::new(UI_FRAME_CAPACITY));
    tx.close();
    let bridge = UiBridge {
        tx,
        root_id: "root".to_string(),
        node: Arc::new(Mutex::new(None)),
        reasoning: Arc::new(Mutex::new(None)),
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), bridge.flush())
        .await
        .expect("通道已关闭时 flush 必须立即返回");
}

// ─────────────────────────────────────────────────────────────────────────
// FrameQueue 的有界 + 增量合并
//
// 这组用例只测 `FrameQueue` 本身，不经 `UiBridge`——`on_delta` 要先有一个节点，
// 而「合并是否无损、是否保序」是队列的性质，与谁在生产它无关。
//
// **慢消费者**是关键输入：`on_delta` 是同步回调（不能 await），所以慢消费者永远
// 无法把背压传回生产者——生产者唯一能做的就是**有界**。因此判据不是「会不会阻塞」，
// 而是「队列满时发生什么」：
//
//  1. 队列长度**不超过**容量（内存有上界）；
//  2. 正文**零丢失**（增量合并而非丢帧）——丢一帧就是界面少一截；
//  3. **终态帧必达**——`Finalize` 永不被丢，丢了正文节点会永远转在「流式中」；
//  4. **保序**——`Finalize` 必须排在它自己那条增量**之后**；
//  5. **屏障帧必达且必回执**——`flush` 的同步点不能被有界化吃掉；
//  6. **关闭语义**——bridge drop 后接收端靠 `pop` 返回 `None` 退出。
//
// ⚠️ 函数名只用 ASCII 标识符：中文全角括号与冒号不是合法的 Rust 标识符字符
// （第一版写成 `fn 慢消费者：…（…）()` 直接编译不过，记在这里免得再犯）。
// ─────────────────────────────────────────────────────────────────────────

fn q() -> FrameQueue {
    FrameQueue::new(UI_FRAME_CAPACITY)
}

/// 排干队列，返回（拼接后的增量正文 / 帧序里节点的次序，`Finalize` 记作 `<finalize>`）
/// ⚠️ **必须先 `close()`**：`pop` 只在队列关闭时返回 `None`，不关的话这里会永远
/// 等下去——而且是**静默**地等（不 panic、不超时、不占 CPU）。第一版就踩了：
/// 整条 `cargo test` 挂住 13 分钟，最后是靠「测试进程 CPU 恒为 0」这件事发现的，
/// 而不是靠任何报错。所以两件事一起做：先关（给循环一个终点），再套 `timeout`
/// （万一将来回归成挂死，几秒内报出来，而不是把门禁拖到超时）。
fn drained(q: FrameQueue) -> (String, Vec<String>) {
    let mut text = String::new();
    let mut order = Vec::new();
    q.close();
    // `enable_time()` 不能省：`tokio::time::timeout` 需要一个**启了 timer 的**
    // runtime，而 `new_current_thread()` 默认不启——不显式开就 panic 在
    // `timeout(...)` 那一行，报错是「no timer running」，与「队列排不干」毫无关系，
    // 极易误诊成队列的 bug（第一版就踩了：4 条测试瞬间红、0.03s，跟超时无关）。
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(f) = q.pop().await {
                match f {
                    UiFrame::Delta { id, text: t } => {
                        text.push_str(&t);
                        order.push(id);
                    }
                    UiFrame::Finalize { .. } => order.push(String::from("<finalize>")),
                    _ => {}
                }
            }
        })
        .await
        .expect("排干超时 ⇒ pop 在队列关闭后仍不返回 None（队列的退出语义坏了）")
    });
    (text, order)
}

/// 灌 500 条增量把队列灌满（容量 64 ⇒ 一定是满的）
fn flood(q: &FrameQueue, n: usize) {
    for i in 0..n {
        q.send(UiFrame::Delta {
            id: "n1".into(),
            text: format!("x{i}"),
        });
    }
}

#[test]
fn queue_passes_frames_through_in_order_when_not_full() {
    let q = q();
    assert!(q.send(UiFrame::Snapshot {
        id: "n1".into(),
        parent: "root".into(),
        text: "a".into(),
        kind: UiNodeKind::Text,
    }));
    assert!(q.send(UiFrame::Delta {
        id: "n1".into(),
        text: "b".into()
    }));
    assert!(q.send(UiFrame::Finalize {
        id: "n1".into(),
        parent: "root".into(),
        text: "ab".into(),
        kind: UiNodeKind::Text,
    }));
    assert_eq!(q.len(), 3);
    let (_text, order) = drained(q);
    assert_eq!(order, vec!["n1", "<finalize>"], "未满时必须原序透传");
}

#[test]
fn slow_consumer_never_exceeds_the_capacity() {
    let q = q();
    flood(&q, 500);
    assert!(
        q.len() <= UI_FRAME_CAPACITY,
        "队列涨到 {} 条，超过容量 {} ⇒ 内存无上界",
        q.len(),
        UI_FRAME_CAPACITY
    );
    assert_eq!(
        q.soft_overflow(),
        0,
        "不该有软超容（队列里全是可合并的增量）"
    );
}

#[test]
fn slow_consumer_loses_no_text() {
    let q = q();
    // ⚠️ 期望值**逐条拼出来**再逐字节比，不要写成 `500 * 4` 这类算术：
    // `x0`..`x9` 是 2 字节、`x10`..`x99` 是 3 字节、`x100`..`x499` 才是 4 字节，
    // 总长 1890 而非 2000。第一版写成 `500 * 4`，差出来的 110 字节被误读成
    // 「丢了正文」，白查一轮——**测试自己的期望值算错，报错信息就会指向错误的方向**。
    let mut expected = String::new();
    for i in 0..500 {
        expected.push_str(&format!("x{i}"));
    }
    q.send(UiFrame::Delta {
        id: "n1".into(),
        text: String::new(),
    });
    for i in 0..500 {
        let piece = format!("x{i}");
        q.send(UiFrame::Delta {
            id: "n1".into(),
            text: piece,
        });
    }
    let (text, _order) = drained(q);
    assert_eq!(
        text.len(),
        expected.len(),
        "正文少了 {} 字节 ⇒ 有增量被丢掉而不是被合并",
        expected.len() as isize - text.len() as isize
    );
    assert_eq!(text, expected, "正文内容与逐条入队的完全一致");
    assert!(text.contains("x499"), "最后一条增量不见了");
    assert!(text.contains("x0"), "第一条增量不见了");
}

#[test]
fn slow_consumer_still_delivers_the_finalize_frame() {
    let q = q();
    flood(&q, 500);
    q.send(UiFrame::Finalize {
        id: "n1".into(),
        parent: "root".into(),
        text: "done".into(),
        kind: UiNodeKind::Text,
    });
    let (_text, order) = drained(q);
    assert!(
        order.contains(&String::from("<finalize>")),
        "队列满时 `Finalize` 被丢了 ⇒ 节点永远定格不了"
    );
}

#[test]
fn finalize_stays_after_its_own_deltas_when_the_queue_is_full() {
    let q = q();
    flood(&q, 500);
    q.send(UiFrame::Finalize {
        id: "n1".into(),
        parent: "root".into(),
        text: "done".into(),
        kind: UiNodeKind::Text,
    });
    let (_text, order) = drained(q);
    let last_delta = order.iter().rposition(|x| x == "n1");
    let finalize_at = order.iter().position(|x| x == "<finalize>");
    assert!(last_delta.is_some() && finalize_at.is_some());
    assert!(
        last_delta.unwrap() < finalize_at.unwrap(),
        "定格帧跑到了增量前面 ⇒ 顺序错乱：{:?}",
        order
    );
}

#[test]
fn barrier_survives_a_full_queue_and_is_acknowledged() {
    let q = Arc::new(q());
    flood(&q, 500);
    let (done, wait) = tokio::sync::oneshot::channel();
    assert!(q.send(UiFrame::Barrier(done)), "屏障帧入队失败");
    let q2 = q.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async move {
        let t = tokio::spawn(async move {
            while let Some(f) = q2.pop().await {
                if let UiFrame::Barrier(d) = f {
                    let _ = d.send(());
                    break;
                }
            }
        });
        wait.await.expect("屏障未回执 ⇒ flush 的同步点失效");
        t.await.unwrap();
    });
}

#[test]
fn closed_queue_rejects_sends_and_drains_to_none() {
    let q = q();
    q.send(UiFrame::Delta {
        id: "n1".into(),
        text: "a".into(),
    });
    q.close();
    assert!(
        !q.send(UiFrame::Delta {
            id: "n1".into(),
            text: "b".into()
        }),
        "关闭后 send 应失败"
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        // 已排队的那条要先吐出来，之后才是 None
        assert!(matches!(q.pop().await, Some(UiFrame::Delta { .. })));
        assert!(
            q.pop().await.is_none(),
            "关闭后 pop 应返回 None（接收端靠这个退出）"
        );
    });
}

// ==================== 压缩寄存 → 会话动作（执行半边） ====================
//
// 压缩跑在步骤 3 / 步骤 6，两处都**不持写者令牌**——落格因此是**会话动作**
// （`session.consolidate`，写方是持令牌的 session）。驱动器只负责两件事：
// 把「压成了什么」按顺序交过去（在 `(turn_no, resume_anchor)` 之后、运行器之前），
// 以及轮号读快照带回来的水位。
//
// **落格幂等 / 归档 / 完整史拼接**是会话存储层的语义，由
// `session/orchestrator/consolidate.test.rs` 钉住；本组只钉执行链这一半。

/// 一份「压缩后的整份消息列表」：摘要一条（真实链路是「摘要 + 保留区」，判据只
/// 关心 **id 序列**——快照的语义是「这份列表**取代**了它之前的一切」）。
fn consolidated(ids: &[&str]) -> Vec<cm::ChatNode> {
    ids.iter()
        .map(|id| cm::ChatNode {
            id: (*id).to_string(),
            principal: Some(principal_for(cm::NodeRole::User)),
            msg_type: Some(cm::NodeKind::Body),
            content: Some(cm::MessageContent::Text(format!(
                "[CONTEXT SNAPSHOT]\n{id}"
            ))),
            status: Some(cm::NodeStatus::Completed),
            ..Default::default()
        })
        .collect()
}

/// 跑一轮（无工具、成功）。
async fn run_one_turn(fx: &Fixture, sink: &symbio_core::ExecEventSink, turn: u64) {
    let text = format!("第 {turn} 轮问题");
    drive_turn(tool_free_req(
        fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        sink,
        symbio_core::ExecAbortSignal::new(),
        turn,
        &text,
    ))
    .await
    .expect("无工具轮应成功");
}

/// 压缩动作的**转发**与**顺序**：执行口把寄存的整份列表原样交给会话端口，且
/// 发生在运行器之前（快照先于本轮的执行事实）。
///
/// ## 判据与反向自检
///
/// ① 端口收到的 id 序列与寄存值逐字相同（驱动器不改写压缩结果）；
/// ② 快照事件排在本轮**派发格之前**——压缩仍是本轮执行的前置动作，归档把旧史
/// 整段移走。用户格先于快照落（它必须先于压缩判决），随归档段保留在
/// 完整史里；派发 / 阶段 / 收束格都排在快照之后，现行窗口不夹旧事件。
///
/// 反向自检：去掉 `drive_turn` 里的 `session.consolidate` 调用 ⇒ ① 红；
/// 把它挪到运行器之后 ⇒ ② 红。
#[tokio::test]
async fn consolidation_is_forwarded_before_the_turn_runs() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    // 第一轮：落 user + final——这就是「被取代的前缀」。
    run_one_turn(&fx, &sink, 0).await;

    // 第二轮：带上「本轮之前压过一次」的寄存位。
    let compacted = consolidated(&["snap-1"]);
    let mut req = tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        1,
        "第二轮",
    );
    req.consolidation = Some(compacted.as_slice());
    drive_turn(req).await.expect("第二轮成功");

    // ── ① 转发：整份列表原样交给会话动作，一次 ──
    let got = fx.port.consolidated.lock().unwrap();
    assert_eq!(got.len(), 1, "本轮只发起一次压缩动作");
    let ids: Vec<&str> = got[0].iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["snap-1"], "驱动器不改写压缩结果");

    // ── ② 顺序：快照先于本轮派发格，用户格在快照之前 ──
    let all = fx.facts.range(Seq::new(0));
    let snap_at = all
        .iter()
        .position(|e| e.kind == symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED)
        .expect("快照应已由会话动作落格");
    let second_user_at = all
        .iter()
        .position(|e| e.kind == EVENT_USER_MESSAGE && e.turn == 1)
        .expect("第二轮的用户格应已落");
    let dispatch_at = all
        .iter()
        .position(|e| e.kind == symbio_plugin_api::events::EVENT_TURN_DISPATCH && e.turn == 1)
        .expect("第二轮的派发格应已落");
    assert!(
        second_user_at < snap_at,
        "用户格先于压缩快照（用户格先于任何判决）"
    );
    assert!(
        snap_at < dispatch_at,
        "快照必须早于本轮派发（压缩是执行的前置动作）：snapshot={snap_at} dispatch={dispatch_at}"
    );
}

// ==================== ：快照之后的恢复锚（D-085） ====================
//
// 判据：一轮「已开未收束」被压缩快照取代、移出 live 段之后，恢复请求仍拿到它的
// `(turn, 用户格 seq)`——而不是 `InternalError`。轮内 / 轮外两条落格路各一条。
//
// 这里用**值注入**复刻「快照已落」：会话动作把快照事件追进事实轨，归档是会话
// 存储层的实现细节（它不改变 `range(0)` 的完整史语义）。本组只钉执行链读的是
// **完整史**而不是现行窗口。

/// 手工开一轮、**不落收束格**——「等待用户动作」在事实源里的形状。
fn open_turn_without_closure(fx: &Fixture, turn: u64, text: &str) -> u64 {
    fx.facts
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                "user",
            )
            .with_payload(serde_json::json!({ "text": text, "tier": "deep" })),
        )
        .expect("开轮格")
        .value()
}

/// 某轮的收束格（final / fallback）——与 `last_open_turn` 的收束口径一致。
fn closure_of(all: &[Event], turn: u64) -> &Event {
    all.iter()
        .find(|e| {
            e.turn == turn
                && matches!(
                    e.kind.as_str(),
                    EVENT_CHAT_ASSISTANT_FINAL | EVENT_CHAT_ASSISTANT_FALLBACK
                )
        })
        .expect("该轮应有收束格")
}

/// **轮外路**：`retry_compaction` 在循环之前落快照并归档，把等待轮的用户格
/// 移出 live 段；随后进来的恢复请求仍要拿到它作锚。
///
/// 反向自检：把调用点改回 `last_open_turn(current_window(&snapshot))` ⇒ 本用例在
/// `drive_turn` 处报「续写轮找不到未收束的轮次」。
#[tokio::test]
async fn an_open_turn_archived_outside_the_turn_is_still_the_resume_anchor() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    // 等待审批的第 0 轮：只有用户格。
    let user_seq = open_turn_without_closure(&fx, 0, "请批准这次写入");

    // 轮外落格（`retry_compaction` 的落点）：快照取代它之前的一切。
    fx.port
        .call(
            run_ops::CONSOLIDATE,
            serde_json::to_value(ConsolidateRequest {
                messages: consolidated(&["snap-r"]),
            })
            .expect("测试载荷"),
        )
        .await
        .expect("轮外压缩动作");

    let all = fx.facts.range(Seq::new(0));
    let snapshot_at = all
        .iter()
        .rposition(|e| e.kind == symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED)
        .expect("快照应已落格");
    let u0_at = all
        .iter()
        .position(|e| e.kind == EVENT_USER_MESSAGE && e.turn == 0)
        .expect("u-0 应在完整史里");
    assert!(
        u0_at < snapshot_at,
        "前提：u-0 已被快照取代（否则本用例测的不是归档后的形状）"
    );
    assert_eq!(
        last_open_turn(&all),
        Some((0, user_seq)),
        "完整史里它仍是已开未收束——恢复锚的判据本身（ADR-085）"
    );

    let mut req = open_turn_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        user_seq,
    );
    // 续写幕的锚 = 恢复阶段事实（生产由 `chat_loop::begin_drive` 落）：它把幕锚
    // 从「归档段里的用户格」挪到这一条 ⇒ 幕锚轮号回到第 0 轮。
    req.drive_seq = land_resume_anchor(&fx, 0, user_seq, "approve");
    drive_turn(req)
        .await
        .expect("归档之后恢复请求应拿到锚点，而不是 InternalError");

    let all = fx.facts.range(Seq::new(0));
    assert_eq!(
        closure_of(&all, 0).produced_by,
        Some(user_seq),
        "收束记在等待中的那一轮、溯源仍是归档段里的用户格（I2）"
    );
    assert_eq!(
        all.iter()
            .filter(|e| e.turn == 0 && e.kind == EVENT_USER_MESSAGE)
            .count(),
        1,
        "续写不重开用户格"
    );
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
}

/// **轮内路**：等待轮之上跑过另一个执行轮，压缩寄存在它自己的执行口落格并
/// 归档——随后到来的恢复请求仍拿到原轮的锚。
///
/// 反向自检同轮外路：调用点退回现行窗口 ⇒ 此处 `drive_turn` 报
/// 「续写轮找不到未收束的轮次」。
#[tokio::test]
async fn an_open_turn_archived_by_the_next_turn_is_still_the_resume_anchor() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    // 等待审批的第 0 轮：只有用户格。
    let user_seq = open_turn_without_closure(&fx, 0, "请批准这次写入");

    // 第 1 轮带着「之前压过一次」的寄存位：它的执行口先落快照，再开自己的
    // 用户格——u-0 因此进了被取代的那段。
    let compacted = consolidated(&["snap-1"]);
    let mut first = tool_free_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        1,
        "第二轮",
    );
    first.consolidation = Some(compacted.as_slice());
    drive_turn(first).await.expect("第 1 轮成功");

    let all = fx.facts.range(Seq::new(0));
    assert!(
        all.iter()
            .any(|e| e.turn == 1 && e.kind == EVENT_USER_MESSAGE),
        "第 1 轮的轮号接着水位走（不是从 0 重号）"
    );
    let snapshot_at = all
        .iter()
        .rposition(|e| e.kind == symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED)
        .expect("快照应已落格");
    let u0_at = all
        .iter()
        .position(|e| e.turn == 0 && e.kind == EVENT_USER_MESSAGE)
        .expect("u-0 应在完整史里");
    assert!(u0_at < snapshot_at, "前提：u-0 已被第 1 轮落下的快照取代");
    assert_eq!(
        last_open_turn(&all),
        Some((0, user_seq)),
        "第 1 轮已收束，完整史里唯一的未收束轮仍是第 0 轮"
    );

    let mut req = open_turn_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        0,
        user_seq,
    );
    // 续写幕的锚 = 恢复阶段事实（生产由 `chat_loop::begin_drive` 落）：第 1 轮的
    // 用户格更晚，但幕锚看的是**恢复格**⇒ 幕锚轮号回到被续写的第 0 轮。
    req.drive_seq = land_resume_anchor(&fx, 0, user_seq, "approve");
    drive_turn(req)
        .await
        .expect("归档之后恢复请求应拿到锚点，而不是 InternalError");

    let all = fx.facts.range(Seq::new(0));
    assert_eq!(
        closure_of(&all, 0).produced_by,
        Some(user_seq),
        "收束记在等待中的那一轮、溯源仍是归档段里的用户格（I2）"
    );
    assert!(
        check_all(guard_vocab(), &all).is_empty(),
        "{:?}",
        check_all(guard_vocab(), &all)
    );
}

// ==================== 判据 12 的生产形状（口径 ①）====================
//
// `loop_rows.test.rs` 那组用手造切片钉住「`now` 只从幕锚来」这条规则本身；本组证明
// **真跑过一轮的会话**在**生产句柄**（`loop_rows::table()`，会话开幕拿的那一个）上
// 重算，账目逐字不变。两次调用之间日志从存储层重取——等值因此不是同参重跑的恒真式，
// 它检查的是求解不夹带内存状态。空账上的「逐字相同」什么都测不到，所以先钉非空。

#[tokio::test]
async fn the_production_table_owes_the_same_rows_on_a_real_log() {
    let fx = setup_full();
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = symbio_core::ExecEventSink::direct(frames.clone());

    // 真实日志由生产写方落格：这一幕只有用户格，`announce-model` 还欠着。
    let anchor = open_turn_without_closure(&fx, 1, "秋天是什么");
    let table = super::super::loop_rows::table().expect("生产行表装载得起来");
    let start = || {
        let log = fx.facts.range(Seq::new(0));
        let at = log
            .iter()
            .position(|e| e.turn == 1)
            .expect("本轮应有事实格");
        log[at..].to_vec()
    };
    let slice = start();
    let now = super::super::loop_rows::wake_now_ms(&slice, anchor);
    let solve = |log: &[Event], frozen: &Arc<Frozen>| {
        symbio_core::ring::owed(super::super::loop_rows::ROWS, frozen, log, now)
    };

    let once = solve(&slice, &table);
    assert!(
        once.iter().any(|d| d.decl.name == "announce-model"),
        "跑过开轮的真实日志上 `announce-model` 该欠着——空账上的「逐字相同」什么都测不到：{once:?}"
    );

    // 日志从存储层**重取**再算一遍：等值检查的是求解不夹带内存状态。
    assert_eq!(
        format!("{once:?}"),
        format!("{:?}", solve(&start(), &table)),
        "同一份日志、同一个生产句柄，重算必须逐字相同"
    );

    // 生产句柄与「把同一张表重新装载一遍」必须给同一笔账——`table()` 的 seal 不夹带过滤。
    let reloaded = super::super::loop_rows::load(super::super::loop_rows::ROWS)
        .expect("同一张表重新装载应通过装载期检查");
    assert_eq!(
        format!("{once:?}"),
        format!("{:?}", solve(&slice, &reloaded)),
        "开幕用的那张表与重新装载的那张表必须是同一张"
    );

    // 账还得掉：续写把这一轮跑到收束，同一张表、同一个幕锚上 `announce-model` 不再欠。
    drive_turn(open_turn_req(
        &fx,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        symbio_core::ExecAbortSignal::new(),
        1,
        anchor,
    ))
    .await
    .expect("续写轮应跑完");
    assert!(
        !solve(&start(), &table)
            .iter()
            .any(|d| d.decl.name == "announce-model"),
        "本轮已收束，`announce-model` 不该还欠着"
    );
}

/// 收工判据（D-123 决策 5 的 `ends_the_drive`）：账本上哪一格让本轮收工。
///
/// ## 它钉的是「一把尺问两个问题」这件事
///
/// core 的停机尺 [`is_stop_fact`] 问的是 C6 的问题——「这一轮为什么停」，两种授权拒绝
/// 都要答（都带 `reason`）。派发器问的是另一个问题：「本轮还往下走吗」。**端口面**的
/// 授权拒绝（`authz.denied` 带 `port`）说的是那一笔工具没放行，载荷回灌给模型后轮还在
/// 走；照搬停机尺会把 auto 幕的一次拒批读成本轮结束（现场 = e2e `t78` 的 auto 幕，
/// 读侧看到的是「工具结果节点根本没长出来」）。
///
/// 反过来，「等待用户」那一格进展**不在**停机尺上（本轮尚未了结，收束格留给恢复幕），
/// 却确实是本轮的最后一步——不补这一格，环会再跑一次模型往返，把「等你回答」跑成
/// 「又答了一轮」。
#[test]
fn the_ledger_decides_which_cell_ends_the_drive() {
    use serde_json::Value;
    use symbio_core::EVENT_SENT;
    use symbio_plugin_api::events::{EVENT_ARTIFACT_ADDED, EVENT_AUTHZ_DENIED};

    let cell = |kind: &'static str, entity: Entity, verb: Verb, turn: u64, payload: Value| {
        Event::pending(
            format!("c-{kind}-{turn}"),
            kind,
            entity,
            verb,
            turn,
            "session/main",
        )
        .with_payload(payload)
    };
    let ends = |e: &Event| ends_the_drive(e, 7);

    // 收束格 / 兜底格：本轮了结。
    assert!(ends(&cell(
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        7,
        Value::Null
    )));
    assert!(ends(&cell(
        EVENT_CHAT_ASSISTANT_FALLBACK,
        Entity::Turn,
        Verb::Closed,
        7,
        Value::Null
    )));
    // 别的轮的收束格不算本轮的工（切片按轮，但判据自己也得站得住）。
    assert!(!ends(&cell(
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        6,
        Value::Null
    )));

    // 收束闸被拒：不落收束格，但它就是本轮的停。
    assert!(ends(&cell(
        EVENT_AUTHZ_DENIED,
        Entity::Turn,
        Verb::Asserted,
        7,
        serde_json::json!({ "kind": "speech", "reason": "denied" })
    )));
    // 端口面被拒：那一笔工具没放行，**轮还在走**——这条就是回归现场。
    assert!(!ends(&cell(
        EVENT_AUTHZ_DENIED,
        Entity::Turn,
        Verb::Asserted,
        7,
        serde_json::json!({ "port": "tool", "id": "call_1", "reason": "denied" })
    )));

    // 等待用户：不是停机格，但是本轮最后一步。
    assert!(ends(&cell(
        EVENT_CHAT_ASSISTANT_PROGRESS,
        Entity::Turn,
        Verb::Progressed,
        7,
        serde_json::json!({ "status": "pending" })
    )));
    // 中途汇报 / 开场白那类进展格：轮还在走。
    assert!(!ends(&cell(
        EVENT_CHAT_ASSISTANT_PROGRESS,
        Entity::Turn,
        Verb::Progressed,
        7,
        serde_json::json!({ "status": "running" })
    )));
    // 产物格：算结算不算停（`closes_turn` 与停机尺不是一把尺，D-067）。
    assert!(!ends(&cell(
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        7,
        Value::Null
    )));
    // 协议格没人拿来收工。
    assert!(!ends(&cell(
        EVENT_SENT,
        Entity::Control,
        Verb::Held,
        7,
        serde_json::json!({ "port": "model", "id": "m-1" })
    )));
}
