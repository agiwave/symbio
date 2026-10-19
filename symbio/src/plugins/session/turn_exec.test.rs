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

use crate::plugins::session::store::EventWalStore;
use std::sync::{Arc, Mutex};

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::{
    check_all, CapabilityMeta, ExecTranscriptWriter, PluginSimpleRequest, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL,
};
use crate::symbio_core::{
    Capability, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequestExt, Registry,
    CAPABILITY_VISITOR,
};

use super::super::config::SessionConfig;
use super::super::store::{current_window, SessionStore};
use super::super::types::Session;

/// 测试请求上下文（工具轮才需要能力注册表；无工具轮一个空信封足够）。
fn test_ctx() -> Arc<dyn crate::symbio_core::PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 带**注册面**的请求上下文（T-158）：把 `tools` 注册进 `CAPABILITY_VISITOR`。
///
/// 三道按名闸门（淘汰 / 承诺记账 / 任务表）的管辖判据现在从注册面读自声明位，
/// ctx 里没有注册面 = 「没有任何工具声明过任何位」。生产里这个注册面由会话
/// 插件的能力收集装上（`capabilities.rs`），测试照同一形状喂——`tools` 传什么，
/// 注册面就是什么，两边不会漂。
async fn test_ctx_declaring(
    tools: &[CapabilityMeta],
) -> Arc<dyn crate::symbio_core::PluginInvokeRequest> {
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
        _ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::Value::Null)
    }
}

/// **轮次事实**（这一轮说了什么 / 跑出了什么）——记忆与学习是**派生副作用**，
/// 不在其中（见 `full_turn_lands_memory_and_learning_facts`）。
///
/// 断言轮次事实时先按它筛一遍：网格里同时住着两类事件，用「总数等于几」去断言
/// 会把两类混在一起——派生素化多写一条就假红，少写一条又假绿。
fn turn_facts(e: &Event) -> bool {
    matches!(
        e.kind.as_str(),
        EVENT_USER_MESSAGE
            | EVENT_ASSISTANT_FINAL
            | EVENT_ASSISTANT_FALLBACK
            | crate::symbio_core::EVENT_ASSISTANT_PROGRESS
            | crate::symbio_core::EVENT_ARTIFACT_ADDED
    )
}

/// 无工具轮的入参包（本文件的三个用例都是这一形态）。
fn tool_free_req<'a>(
    session: &'a PersistentChatSession,
    provider: Arc<dyn ModelProvider>,
    sink: &'a crate::symbio_core::ExecEventSink,
    abort: crate::symbio_core::ExecAbortSignal,
) -> ExecTurn<'a> {
    ExecTurn {
        session,
        provider,
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx(),
        system_prompt: "system",
        abort,
        root_id: "r-1",
        sink,
        user_text: "你好",
        user_node_id: None,
        window_turns: 6,
        tools: &[],
        resume: None,
        recalled: None,
        skill_obs: &[],
        // 快路候选集默认空 ⇒ 与接线前逐字同路（`run_with_tools`）。
        skill_hits: &[],
        // 请求级前缀：测试不走请求视图层（三段皆空）。
        prefix: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        user_meta: None,
        on_round: None,
        // 开场白（T-056）：本夹具不产生（`Escalate` 首响由 chat_loop 递入）。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: super::super::config::PresentationTree::Grouped,
    }
}

/// 帧收集口（测试的 ExecEventSink 出口）。
struct CollectingFrames(Mutex<Vec<cm::ChatNode>>);

#[async_trait::async_trait]
impl ExecTranscriptWriter for CollectingFrames {
    async fn apply(&self, m: cm::ChatNode) {
        self.0.lock().unwrap().push(m);
    }
}

/// 补充成为事实、并在**下一轮**仍可见（缺口 3 的真正判据，插件层那一半）。
///
/// ## 为什么这条在插件层而不在 core
///
/// core 只提供「工具循环里什么时候问一次」这个挂点（`TurnInput::inject`）；
/// 「什么是补充」「它怎么落成 `turn.supplemented`」全在 `plugins/session`（收件箱、
/// `merge_supplements`、`EventWalStore`）。core 那侧的判据是
/// `injected_messages_reach_the_next_request_of_the_same_turn`（只验「进了本轮请求」）。
///
/// ## 三段各只能证明一件事
///
/// - **落格**（WAL 有 `turn.supplemented` + 溯源 + `count`）：它成为了事实；
/// - **回读下一次请求**：它在**本轮**就对模型可见（用户就是在这轮说的话）；
/// - **重开 WAL 再投影**：它在**下一轮**仍可见——这才是缺口 3 的原始症状
///   （只进消息列表、不进事实网格的补充，下一轮就从模型眼前消失）。
///
/// ## 反向自检
///
/// 把闭包里的 `store.append` 去掉 ⇒ 第 1、3 段红（网格里没有它）；
/// 把 `store.append` 保留但返回空 vec ⇒ 第 2 段红（本轮请求里没有它）。
#[tokio::test]
async fn supplements_become_facts_visible_next_turn() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
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
            let mut once = fired.lock().unwrap();
            if *once {
                return None;
            }
            *once = true;
            Some(super::super::transcript::supplements::DrainedSupplement {
                text: "顺便也看看 README".into(),
                count: 2,
                id: "sup-1".into(),
                ids: vec!["sup-1".into(), "sup-2".into()],
            })
        }) as super::SupplementFn
    };

    execute_turn(ExecTurn {
        session: &session,
        provider: Arc::new(provider),
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx(),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        user_node_id: None,
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            keywords: vec![],
            category: None,
            examples: None,
            ..Default::default()
        }],
        resume: None,
        recalled: None,
        skill_obs: &[],
        skill_hits: &[],
        prefix: None,
        consolidation: None,
        supplements: Some(supplements),
        supplemental_no: Default::default(),
        user_meta: None,
        on_round: None,
        // 开场白（T-056）：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: super::super::config::PresentationTree::Grouped,
    })
    .await
    .expect("工具轮执行成功");

    // ── ① 落格：WAL 里有 `turn.supplemented`，带溯源与 `count` ──────────
    //
    // 句柄必须在 `execute_turn` 之后**重开**（写者令牌是独占的）——顺带满足
    // 「不拿内存里的快照糊弄」：重开读的就是落盘状态。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("重开 WAL");
    let snap = store.range(Seq::new(0));
    let sup = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_TURN_SUPPLEMENTED)
        .unwrap_or_else(|| panic!("补充必须落成事实（否则下一轮就消失——那正是缺口 3）：{snap:?}"));
    assert_eq!(
        sup.entity,
        crate::symbio_core::Entity::Turn,
        "落在 Turn 实体上"
    );
    assert_eq!(sup.verb, crate::symbio_core::Verb::Asserted, "断言类动词");
    assert_eq!(
        sup.payload.get("count").and_then(|v| v.as_u64()),
        Some(2),
        "原条数要原样保留（2 条合成 1 段）"
    );
    let user_seq = snap
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_USER_MESSAGE)
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

    // ── ③ 下一轮可见：**重开 WAL** 再投影（不拿内存里的快照糊弄） ──────────
    let reopened = crate::plugins::session::store::EventWalStore::open(
        dir.join(super::super::paths::V2_WAL_FILE),
    )
    .expect("重开 WAL");
    let projected = crate::symbio_core::transcript()
        .apply(
            &reopened.range(crate::symbio_core::Seq::new(0)),
            i64::MAX,
            crate::symbio_core::Budget::generous(),
        )
        .value;
    assert!(
        projected
            .entries
            .iter()
            .any(|e| e.role == "user" && e.text.contains("README")),
        "补充必须在**下一轮**仍可见（重开 WAL 后投影）：{projected:?}"
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
    /// 记的是 `execute_turn` 真正收到的 `&[ChatNode]` —— 即**线格式那一侧**，
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
    async fn execute_turn(
        &self,
        _system: &str,
        messages: &[cm::ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        env: &crate::symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        // 记下本次请求的**全部消息**（角色 + 正文）——工具轮的硬契约是「工具结果
        // 进了下一次请求」，那条断言只能回读请求体（不变量/网格都证明不了）。
        //
        // ⚠️ 记**全部消息**，不只 `messages.first()`：只取首条时「工具结果是
        // `role=tool`」不可断（整段挤在一条 user 消息里也照样绿）。
        let received: Vec<(String, String)> = messages
            .iter()
            .map(|m| {
                // `role` 是 `Option<NodeRole>`：`Debug` 会打成 `Some(user)`，
                // 而断言要的是 `user`。所以 `unwrap_or` 之后再小写——**不要**对
                // 整个 `Option` 做 `to_lowercase`，那会留下 `some(...)` 包装。
                let role =
                    format!("{:?}", m.role.as_ref().unwrap_or(&Default::default())).to_lowercase();
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
            Behavior::Abort => {
                return Err(PluginError::Aborted {
                    partial: Vec::new(),
                })
            }
            Behavior::AbortAfterPartial => {
                // 真实链路的半截形态：推理快照帧 + 正文快照帧先行（首片即快照），
                // 生成被中止在半句话上——帧进桥的累积，收束与出口按累积取半截。
                env.sink()
                    .emit(
                        cm::ChatNode {
                            id: "n-rea".into(),
                            parent_id: Some("r-1".into()),
                            role: Some(cm::NodeRole::Assistant),
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
                        role: Some(cm::NodeRole::Assistant),
                        msg_type: Some(cm::NodeKind::Body),
                        content: Some(cm::MessageContent::Text("让我想想这个名词".into())),
                        status: Some(cm::NodeStatus::Streaming),
                        ..Default::default()
                    })
                    .await;
                return Err(PluginError::Aborted {
                    partial: Vec::new(),
                });
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
                            role: Some(cm::NodeRole::Assistant),
                            msg_type: Some(cm::NodeKind::Body),
                            content: Some(cm::MessageContent::Text("我先查一下。".into())),
                            status: Some(cm::NodeStatus::Streaming),
                            ..Default::default()
                        })
                        .await;
                    return Ok(TurnOutput {
                        text: "我先查一下。".into(),
                        tool_calls: vec![crate::symbio_core::TurnToolCallInfo {
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
                            role: Some(cm::NodeRole::Assistant),
                            msg_type: Some(cm::NodeKind::Body),
                            content: Some(cm::MessageContent::Text("我来委托。".into())),
                            status: Some(cm::NodeStatus::Streaming),
                            ..Default::default()
                        })
                        .await;
                    return Ok(TurnOutput {
                        text: "我来委托。".into(),
                        tool_calls: vec![crate::symbio_core::TurnToolCallInfo {
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
                role: Some(cm::NodeRole::Assistant),
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

/// 持久会话装配（事实源落会话目录）；返回 (会话, 会话目录, 清理句柄)。
async fn setup_full() -> (PersistentChatSession, std::path::PathBuf, tempfile::TempDir) {
    setup_full_with(false).await
}

/// 同 [`setup_full`]，另可开技能编译——步 22 的**编译**半边要它才动
/// （观测半边不需要，`memory.recalled` 与开关无关）。
async fn setup_full_with(
    skill_compile: bool,
) -> (PersistentChatSession, std::path::PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "turn_exec_test".to_string();
    // 先落盘一次空会话，使 session_dir 能解析到实际目录（v2 WAL 的安放处）。
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store.session_dir(&session_id).expect("应返回会话目录");
    let config = SessionConfig {
        skill_compile_enabled: skill_compile,
        ..Default::default()
    };
    let session = PersistentChatSession::new(
        session_id,
        Arc::new(tokio::sync::RwLock::new(config)),
        store,
    );
    (session, dir, tmp)
}

/// 成功轮：事实原生入格（user + final，不变量绿），增量经 UiBridge 回 UI，
/// 全文与分片拼接一致、正文节点 id 稳定。
#[tokio::test]
async fn tool_free_turn_writes_grid_and_streams_ui() {
    let (session, dir, _tmp) = setup_full().await;
    // 收集口：ExecEventSink::direct 的 writer 不可回读，自持一份 Arc 引用帧。
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    let res = execute_turn(tool_free_req(
        &session,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
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

    // UI 帧：首片快照建节点（Streaming），窄帧追加——与 model 插件直发同构。
    let got = frames.0.lock().unwrap();
    assert_eq!(got.len(), 2, "快照 + 窄帧，不多不少：{got:?}");
    assert_eq!(
        got[0].id, out.response_text_child_id,
        "收口用的 id 就是首片建的节点"
    );
    assert_eq!(got[0].status, Some(cm::NodeStatus::Streaming));
    assert_eq!(got[0].parent_id.as_deref(), Some("r-1"));
    assert!(
        matches!(&got[0].content, Some(cm::MessageContent::Text(t)) if t == "秋"),
        "快照帧带首片全文：{:?}",
        got[0].content
    );
    assert_eq!(got[1].id, out.response_text_child_id, "窄帧同一节点");
    assert_eq!(got[1].delta.as_deref(), Some("天好"), "窄帧只带增量");

    // 事实网格：user 格 + final 格，不变量全绿，final 溯源指向本轮用户格。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + final 格：{snap:?}");
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(snap[0].turn, 0, "turn 号 = WAL 内 user.message 计数");
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FINAL);
    assert_eq!(snap[1].payload["text"], "秋天好", "final 落的是聚合全文");
    assert!(snap[1].produced_by.is_some(), "final 溯源指向用户格");
}

/// 失败轮（I3）：fallback 格照落（网格不缺格），错误上抛给 chat_loop 走
/// Failed 出口。
#[tokio::test]
async fn tool_free_turn_failure_leaves_fallback_event_and_bubbles() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    let err = execute_turn(tool_free_req(
        &session,
        Arc::new(FaithfulProvider::new(Behavior::Fail).0),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    ))
    .await
    .err()
    .expect("失败必须上抛（Failed 出口）");

    assert!(
        matches!(err, PluginError::InternalError(ref m) if m.contains("模型断了")),
        "上抛的是生成失败本体：{err:?}"
    );

    // 网格里用户格 + fallback 格都在——失败也是一句话，不是静默。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + fallback 格：{snap:?}");
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FALLBACK);
    let why = snap[1].payload["why"].as_str().unwrap_or_default();
    assert!(why.contains("模型断了"), "兜底格要记失败原因：{why}");
}

/// 中止轮：**照落收束格**（`chat.assistant.final`，载荷 `status: "aborted"` +
/// `terminated`），上抛 `Aborted` 走独立出口——用户自己按的停止不该有错误条与
/// 重试入口，也不进兜底分子；本轮无半截产出时 `partial` 为空。
#[tokio::test]
async fn tool_free_turn_abort_closes_the_cell_and_bubbles_aborted() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    let err = execute_turn(tool_free_req(
        &session,
        Arc::new(FaithfulProvider::new(Behavior::Abort).0),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    ))
    .await
    .err()
    .expect("中止必须上抛（Aborted 出口，不是 Failed）");

    let PluginError::Aborted { partial } = &err else {
        panic!("上抛的是中止本体，不被压成失败：{err:?}");
    };
    assert!(partial.is_empty(), "本轮没有半截产出：{partial:?}");

    // 网格：用户格 + 收束格——中止照落收束格（kind 不换），绝不落兜底格。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + 收束格：{snap:?}");
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FINAL, "中止照落收束格");
    assert_eq!(snap[1].payload["status"], "aborted", "终止是状态字段");
    assert_eq!(
        snap[1].payload["terminated"]["reason"], "aborted",
        "终止原因入格"
    );
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
}

/// 事实完整性（T-052 判据）：**中止后那半句话还在**——模型说到一半（"让我想想
/// 这个名词"）被打断，半截正文 / 半截思考（一）随出口带走（`Aborted { partial }`，
/// 由消费循环 `persist_failure` 收尾入格），（二）照原样入格
/// （收束格载荷 `text` / `reasoning`）。用户随后追问那个名词时，前置上下文
/// （那半句 + 半截思考）在重建的历史里都在。
#[tokio::test]
async fn aborted_half_sentence_survives_in_partial_and_grid() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    let err = execute_turn(tool_free_req(
        &session,
        Arc::new(FaithfulProvider::new(Behavior::AbortAfterPartial).0),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    ))
    .await
    .err()
    .expect("中止必须上抛（Aborted 出口）");

    // ① 随出口带走：正文节点 + 推理节点都进 `partial`（落库镜像的来源）。
    let PluginError::Aborted { partial } = &err else {
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

    // ② 入格：收束格载荷里那半句话与半截思考照原样可查。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "用户格 + 收束格：{snap:?}");
    let cell = snap[1];
    assert_eq!(cell.kind, EVENT_ASSISTANT_FINAL);
    assert_eq!(cell.payload["status"], "aborted");
    assert_eq!(cell.payload["terminated"]["reason"], "aborted");
    assert_eq!(cell.payload["text"], "让我想想这个名词", "半截正文入格");
    assert_eq!(
        cell.payload["reasoning"], "这个名词需要先定义。",
        "半截思考入格（reasoning 在网格里查得到）"
    );
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
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
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::ToolThenAnswer);

    let res = execute_turn(ExecTurn {
        session: &session,
        provider: Arc::new(provider),
        // 无插件宿主 ⇒ 工具分发按「没有父插件」诚实失败——本用例验的是**链路**
        // （落格 / 帧 / 回灌），工具真正跑起来由 e2e 覆盖。失败是信息性的：
        // 结果照样回灌，循环照常继续。
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx(),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        user_node_id: None,
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            keywords: vec![],
            category: None,
            examples: None,
            ..Default::default()
        }],
        resume: None,
        recalled: None,
        skill_obs: &[],
        skill_hits: &[],
        // 请求级前缀：本用例不走请求视图层（三段皆空）。
        prefix: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        user_meta: None,
        on_round: None,
        // 开场白（T-056）：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: super::super::config::PresentationTree::Grouped,
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
    // ADR-048a 的核心断言：工具结果与助手发言**各带各的角色**（只取 `messages.first()`
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
    assert_eq!(tool_result.role, Some(cm::NodeRole::Tool));

    // ── UI 帧：中途正文被**定格**（不是永远流式中），工具节点与结果都在 ────
    let got = frames.0.lock().unwrap();
    let text_frames: Vec<_> = got
        .iter()
        .filter(|m| m.is_text() && m.role == Some(cm::NodeRole::Assistant))
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
            .any(|m| m.role == Some(cm::NodeRole::Tool)
                && matches!(&m.content, Some(cm::MessageContent::Text(t)) if t.contains("No parent plugin"))),
        "工具结果节点必须广播给前端：{got:?}"
    );

    // ── 网格：用户格 + 产物格 + final 格，产物格溯源指向本轮用户格 ────────
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 3, "用户格 + 产物格 + final 格：{snap:?}");
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(snap[1].kind, crate::symbio_core::EVENT_ARTIFACT_ADDED);
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
        "产物格溯源指向本轮用户格（S02 §3 的 caused_by 断言）"
    );
    assert_eq!(snap[2].kind, EVENT_ASSISTANT_FINAL);
}

/// 轮边界汇报「产生即上线」（[T-136-轮中对话面节点不入实时面](../../../../docs/009-task/136-轮中对话面节点不入实时面.md) 的另一半）：
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
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::ToolThenAnswer);

    // 只报一次：`RoundHookFn` 每跑完一轮工具都会被问一次，第二轮起返回 `None`。
    let fired = Arc::new(Mutex::new(false));
    let on_round = {
        let fired = fired.clone();
        Arc::new(move |_supplemented_at: Option<i64>| {
            let fired = fired.clone();
            Box::pin(async move {
                let mut once = fired.lock().unwrap();
                if *once {
                    return None;
                }
                *once = true;
                Some(super::super::round_hook::ReportDraft {
                    text: "已经完成 1 轮工具调用，还在继续。".into(),
                    tool_rounds: 1,
                    quiet_ms: 45_000,
                })
            })
                as std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = Option<super::super::round_hook::ReportDraft>,
                            > + Send,
                    >,
                >
        }) as super::RoundHookFn
    };

    let res = execute_turn(ExecTurn {
        session: &session,
        provider: Arc::new(provider),
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx(),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        user_node_id: None,
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            keywords: vec![],
            category: None,
            examples: None,
            ..Default::default()
        }],
        resume: None,
        recalled: None,
        skill_obs: &[],
        skill_hits: &[],
        prefix: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        user_meta: None,
        on_round: Some(on_round),
        opening_text: None,
        opening_dialog: None,
        presentation_tree: super::super::config::PresentationTree::Grouped,
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
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("重开 WAL");
    let all = store.range(Seq::new(0));
    let reported = all
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_ASSISTANT_REPORTED)
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
            .filter(|e| e.kind == crate::symbio_core::EVENT_ASSISTANT_REPORTED)
            .count(),
        1,
        "只报一次（游标从 0 起 ⇒ 事件 id `rp-{{turn}}-0`）：{all:?}"
    );
}

// ==================== 收束派生事实：记忆与学习（步 11–13 + 步 22）====================

/// **收束派生事实**：记忆三段（步 11–13）与技能观测 / 编译（步 22）必须入格。
///
/// ## 这个用例挡的是什么
///
/// 轮次事实（用户格 / final 格 / 产物格）由 v2 运行器原生记账，而记忆与学习
/// **不是轮次事实**——运行器一处都不写，写方是 `derived_facts::record_learning`。
/// 写方若不被调用（或出参没交回），长期记忆（S06）与技能
/// 自我改进（S11）就**静默全丢**。
///
/// 三条断言各自钉一段：
/// ① 步 11 编码（`memory.encoded{tag:"经验"}`，锚 = 本轮用户格）；
/// ② 步 22 观测（`memory.recalled{skill_id, fallback}`，锚同）——缺它 `calibration`
///    永远读到「零使用」，置信度恒 1.0，回退一次都不会发生（S11 §5 静默失效第 2 行）；
/// ③ 步 22 编译（`memory.encoded{tag:"skill"}`）——缺它下一轮无技能可用。
///
/// **反向自检**：注掉 `turn_exec` 里的 `record_learning` 调用，本用例必须红。
#[tokio::test]
async fn full_turn_lands_memory_and_learning_facts() {
    use crate::symbio_core::{
        EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_RECALLED,
    };

    let (session, dir, _tmp) = setup_full_with(true).await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::Faithful);

    // 技能路由判定由读侧给出（生产里是 `prepare_turn_inputs` 调 `skill_events::route`）。
    let obs = [("sk-1".to_string(), false)];
    let mut req = tool_free_req(
        &session,
        Arc::new(provider),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    );
    req.skill_obs = &obs;
    execute_turn(req).await.expect("full 轮执行成功");

    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let events = store.range(Seq::new(0));
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();

    let user_seq = events
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.seq.map(|s| s.value()))
        .expect("本轮用户格必须入格");

    let encoded: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_ENCODED)
        .collect();
    assert!(
        encoded.iter().any(|e| e.payload["tag"] == "经验"),
        "步 11 编码必须入格：{kinds:?}"
    );
    assert!(
        encoded.iter().any(|e| e.payload["tag"] == "skill"),
        "步 22 技能编译必须入格：{kinds:?}"
    );
    assert!(
        !encoded.is_empty() && encoded.iter().all(|e| e.produced_by == Some(user_seq)),
        "记忆的溯源锚必须是本轮开口（I2）：{encoded:?}"
    );

    let obs_events: Vec<&Event> = events
        .iter()
        // 按**格子**认自己的输入（T-085 批①：校准观测从 `memory.recalled` 搬进
        // `memory.calibrated`）。旧写法按载荷字段判别——那正是本判据要证明不再依赖的。
        .filter(|e| e.kind == EVENT_MEMORY_CALIBRATED)
        .collect();
    assert_eq!(obs_events.len(), 1, "步 22 观测必须逐条入格：{kinds:?}");
    assert!(
        !events
            .iter()
            .any(|e| { e.kind == EVENT_MEMORY_RECALLED && e.payload.get("skill_id").is_some() }),
        "校准观测不得再落在 `memory.recalled` 上——那会让一张格子背两种语义"
    );
    assert_eq!(obs_events[0].payload["skill_id"], "sk-1");
    assert_eq!(obs_events[0].payload["fallback"], false);
    assert_eq!(
        obs_events[0].produced_by,
        Some(user_seq),
        "观测的溯源锚同为本轮开口"
    );
}

// ==================== 收束派生事实：承诺 / 任务表 / 熔断 ====================

/// **收束派生事实**：承诺（`agent_run` 的代际立约）必须入格。
///
/// ## 这个用例挡的是什么
///
/// 承诺的数据来源在工具执行层（`Delegation`，`process_tool_calls_async` 的出参），
/// 写方是 `derived_facts::record_derived`（`turn_exec` 轮末调）。若分发方不把出参交回
/// （`SessionDispatchPort` 的三个出参收在本地 `Vec`、经 `take_derived` 交回，
/// 不收不交就等于丢），承诺就**静默全丢**——代际立约整体失效。
///
/// 本用例走的是**真实通道**：假 provider 回一个 `agent_run` 工具调用 ⇒
/// `SessionDispatchPort::dispatch` 收集出参 ⇒ `execute_turn` 轮末调 `record_derived`
/// 落格。它同时钉住两件事——出参通道（`take_derived`）与写方（`record_derived`）——
/// 以及锚点：立约溯源必须是本轮开口（I2）。
///
/// **反向自检**：把 `tool_dispatch` 里三份出参改回 `&mut Vec::new()`，本用例必须红。
#[tokio::test]
async fn full_turn_lands_derived_commitment_facts() {
    use crate::symbio_core::{EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OFFERED};

    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, _prompts) = FaithfulProvider::new(Behavior::ToolThenAgentRun);

    // 注册面与模型面同一份元数据（T-158）：`agent_run` 自声明「派活 + 立约」两位，
    // 承诺记账只认声明——与生产（`plugins/agent/host/subagent.rs`）逐字同形。
    let tool_meta = CapabilityMeta {
        name: "agent_run".into(),
        description: "派生一个子智能体".into(),
        input_schema: serde_json::json!({ "type": "object" }),
        keywords: vec![],
        category: None,
        examples: None,
        provides_assign_work: true,
        records_delegation_promise: true,
        ..Default::default()
    };

    let res = execute_turn(ExecTurn {
        session: &session,
        provider: Arc::new(provider),
        // 无插件宿主 ⇒ `agent_run` 诚实失败（"No parent plugin"）。失败同样是
        // **没履约**——立约与违约都要入格（S08 §5：违约可被观测是 T5 的前提）。
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx_declaring(std::slice::from_ref(&tool_meta)).await,
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "请委托评审",
        user_node_id: None,
        window_turns: 6,
        tools: std::slice::from_ref(&tool_meta),
        resume: None,
        recalled: None,
        skill_obs: &[],
        skill_hits: &[],
        // 请求级前缀：本用例不走请求视图层（三段皆空）。
        prefix: None,
        consolidation: None,
        supplements: None,
        supplemental_no: Default::default(),
        user_meta: None,
        on_round: None,
        // 开场白（T-056）：本夹具不产生。
        opening_text: None,
        opening_dialog: None,
        presentation_tree: super::super::config::PresentationTree::Grouped,
    })
    .await
    .expect("工具轮执行成功");
    assert_eq!(res.output.text, "秋天好", "收尾轮的正文");

    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let events = store.range(Seq::new(0));
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();

    let user_seq = events
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.seq.map(|s| s.value()))
        .expect("本轮用户格必须入格");

    let offered = events
        .iter()
        .find(|e| e.kind == EVENT_COMMITMENT_OFFERED)
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
        "无宿主 ⇒ 委托失败 ⇒ 违约必须可观测（S08 §5）：{kinds:?}"
    );
    assert!(check_all(&events).is_empty(), "{:?}", check_all(&events));
}

// ==================== 反射档（S11 快路的执行半边）====================

/// 命中一条已编译技能 ⇒ 本轮**一次模型都不调**，以技能正文收束；开轮格声明
/// `reflex`，收束格 `cost_ms` 在反射档预算内。
///
/// ## 判据只有一个，就是"没调模型"
///
/// `prompts` 为空是本用例最硬的一条。技能命中若只是"把技能正文当提示词喂给模型"，
/// 请求数照样是 1——那样"跳过模型"就是一句空话，S11 §6.2 的加速根本不存在。
/// 其余三条（`tier = reflex` / `model = reflex` / `cost_ms` 在预算内）钉的是**埋点**：
/// 没有它们，"跳过"发生了却无从观测。
///
/// ## 为什么"只对新开轮"这条边界要在这里立
///
/// 续写轮的 `user_text` 仍是原轮那句发言（用户没再说话）——若不禁，同一句话命中技能
/// 时反射档会去新开一个 `u-{turn}` 格，而该轮的用户格**已经在网格里**（等待轮落的）
/// ⇒ 撞幂等键（`AppendError::Duplicate` → Failed 出口），本轮既完不成收束、又变成一次
/// 报错。语义上同样说不通：续写要跑的是刚被批准的那次工具，不是拿技能顶掉它。
/// 该边界由 `turn_exec` 的 `resume_anchor.is_none()` 判定，故此处钉住：`resume` 为
/// `Some` 时即便命中集非空，也照走完整推理（模型被调用）。
#[tokio::test]
async fn skill_hit_takes_the_reflex_tier_without_any_model_call() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::Faithful);

    // `trigger` 与 `tool_free_req` 的 `user_text`（"你好"）逐字相同 ⇒ 命中。
    let hits = [super::super::skill_events::SkillLlmHit {
        skill_id: "sk-1".into(),
        trigger: "你好".into(),
        content: "先列要点；再合并同类项。".into(),
    }];
    let mut req = tool_free_req(
        &session,
        Arc::new(provider),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    );
    req.skill_hits = &hits;
    let res = execute_turn(req).await.expect("反射档执行成功");

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

    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let all = store.range(Seq::new(0));
    let snap: Vec<&Event> = all.iter().filter(|e| turn_facts(e)).collect();
    assert_eq!(snap.len(), 2, "开轮格 + 收束格（没有产物格）：{snap:?}");
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(
        snap[0].payload["tier"], "reflex",
        "开轮格声明反射档（S11 §3 的档位观测；兜底率按层统计靠它）"
    );
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FINAL);
    assert_eq!(snap[1].payload["text"], "先列要点；再合并同类项。");
    assert_eq!(
        snap[1].payload["model"], "reflex",
        "载荷的 `model` 记产者——空着会让命中轮与普通轮在事件面上无从区分"
    );
    assert!(
        snap[1].cost_ms <= LatencyTier::Reflex.budget_ms(),
        "命中后耗时必须在反射档预算内（S11 §6.2：命中显著低于未命中）：{}",
        snap[1].cost_ms
    );
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
}

/// 反向（边界）：命中集非空但本轮是**续写轮** ⇒ 一律走完整推理，技能不得顶掉
/// 刚被批准的那次工具。
#[tokio::test]
async fn a_skill_hit_never_takes_over_a_resumed_turn() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let (provider, prompts) = FaithfulProvider::new(Behavior::Faithful);

    // 续写轮的前提是「本轮已开未收束」（`last_open_turn` 要能找到它）——先手工开一轮。
    // 句柄必须**在 `execute_turn` 之前释放**：写者令牌是独占的（`wal.rs`），
    // 本测试握着它，`execute_turn` 里那次 `open` 就只能拿到只读降级、落格全失败。
    {
        let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
        store
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
            .expect("开轮格");
    }

    let hits = [super::super::skill_events::SkillLlmHit {
        skill_id: "sk-1".into(),
        trigger: "你好".into(),
        content: "先列要点；再合并同类项。".into(),
    }];
    let mut req = tool_free_req(
        &session,
        Arc::new(provider),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    );
    req.skill_hits = &hits;
    req.resume = Some(ResumedTool {
        name: "vdfs_read".into(),
        args: serde_json::json!({ "path": "a.md" }),
        text: "文件内容：hello".into(),
        // 节点身份（调用 / 轮根 / 结果）：本用例验的是续写路径与技能不顶替，
        // 不验节点挂靠——留空 ⇒ 回落合成。
        call_id: String::new(),
        turn_id: String::new(),
        result_id: String::new(),
    });

    let res = execute_turn(req).await.expect("续写轮执行成功");
    assert_eq!(
        res.output.text, "秋天好",
        "续写轮走完整推理（模型作答），技能正文不得顶替它"
    );
    assert_eq!(
        prompts.lock().unwrap().len(),
        1,
        "续写轮照常请求模型：{:?}",
        prompts.lock().unwrap()
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
// FrameQueue 的有界 + 增量合并（批 C4 的判据）
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

// ==================== 压缩入格 + 归档（S2.7-b2） ====================
//
// 压缩跑在步骤 3 / 步骤 6，两处都**不持写者令牌**（令牌只在步骤 4 的
// `execute_turn` / 答话轮的 `execute_final_reply_turn` 里拿）——就地落格会拿到
// `NotTheWriter`，而那是**静默**的：压缩照常生效、界面照常变短，只有事实源里
// 少一条事件。于是压缩把「压成了什么」寄存在 `SessionContext::consolidated`，
// 由下一个执行口落格并归档。这一组判据钉的就是那条寄存 → 落格 → 归档的链。

/// 一份「压缩后的整份消息列表」：摘要一条（真实链路是「摘要 + 保留区」，判据只
/// 关心 **id 序列**——快照的语义是「这份列表**取代**了它之前的一切」）。
fn consolidated(ids: &[&str]) -> Vec<cm::ChatNode> {
    ids.iter()
        .map(|id| cm::ChatNode {
            id: (*id).to_string(),
            role: Some(cm::NodeRole::User),
            msg_type: Some(cm::NodeKind::Body),
            content: Some(cm::MessageContent::Text(format!(
                "[CONTEXT SNAPSHOT]\n{id}"
            ))),
            status: Some(cm::NodeStatus::Completed),
            ..Default::default()
        })
        .collect()
}

/// 会话 WAL 的路径（`turn_exec` 与测试同一口径）。
fn wal_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join(super::super::paths::V2_WAL_FILE)
}

/// 跑一轮（无工具、成功），返回可复用的收集口。
async fn run_one_turn(session: &PersistentChatSession, sink: &crate::symbio_core::ExecEventSink) {
    execute_turn(tool_free_req(
        session,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        sink,
        crate::symbio_core::ExecAbortSignal::new(),
    ))
    .await
    .expect("无工具轮应成功");
}

/// 压缩入格 + 归档的**三段判据**：落格（完整史里有快照，格子对）/ 取代（现行窗口
/// 以快照开头、被取代的前缀进了归档段）/ 完整史不丢（`range(0)` 仍含前缀）。
///
/// 「现行窗口」不是第二个读口：它是消费方从完整史里按「最近一份快照在哪」切出来
/// 的那段（[plan/14 §2.2](../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 推论 1）。
///
/// ## 反向自检
///
/// 把 `execute_turn` 里的 `consolidate_into_wal` 调用去掉 ⇒ ① 红（没有快照事件）；
/// 把 `consolidate_into_wal` 末尾的 `archive_before` 去掉 ⇒ ② 红（现行窗口仍以旧
/// 用户格开头、归档目录不存在）。
#[tokio::test]
async fn consolidation_lands_a_snapshot_and_archives_the_prefix() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    // 第一轮：落 user + final——这就是「被取代的前缀」。
    run_one_turn(&session, &sink).await;
    let wal = wal_path(&dir);
    let before = EventWalStore::open(&wal).expect("open wal").len();
    assert!(before >= 2, "第一轮至少落了用户格 + final 格");

    // 第二轮：带上「本轮之前压过一次」的寄存位。
    let compacted = consolidated(&["snap-1"]);
    let mut req = tool_free_req(
        &session,
        Arc::new(FaithfulProvider::new(Behavior::Faithful).0),
        &sink,
        crate::symbio_core::ExecAbortSignal::new(),
    );
    req.consolidation = Some(compacted.as_slice());
    execute_turn(req).await.expect("第二轮成功");

    // ── ① 落格：完整史里恰一条快照，格子 = message × progressed，turn = 0 ──
    let store = EventWalStore::open(&wal).expect("open wal");
    let all = store.range(Seq::new(0));
    let window = current_window(&all);
    let snaps: Vec<&Event> = all
        .iter()
        .filter(|e| e.kind == crate::symbio_core::EVENT_SESSION_CONSOLIDATED)
        .collect();
    assert_eq!(snaps.len(), 1, "恰一条快照：{snaps:?}");
    assert_eq!(snaps[0].entity, Entity::Message);
    assert_eq!(snaps[0].verb, Verb::Progressed);
    assert_eq!(snaps[0].turn, 0, "压缩不属于任何一轮（跨轮边界）");
    assert_eq!(snaps[0].payload["messages"][0]["id"], "snap-1");

    // ── ② 取代：现行窗口以快照开头，被取代的前缀进了归档段 ──
    assert_eq!(
        window[0].kind,
        crate::symbio_core::EVENT_SESSION_CONSOLIDATED,
        "现行窗口以快照开头——被取代的前缀已移走"
    );
    let archive_dir = dir.join("transcripts").join("wal-archive");
    let mut segs: Vec<String> = std::fs::read_dir(&archive_dir)
        .expect("归档段目录应存在")
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    segs.sort();
    assert_eq!(
        segs,
        vec![format!("v2-events.wal.0-{}", before - 1)],
        "段名带首末号，末号 = 被取代前缀的末号（= 落格前的 head − 1）"
    );

    // ── ③ 完整史不丢：`range(0)` = 归档段 + live，差集恰是被取代的前缀 ──
    assert_eq!(
        all.len(),
        store.len() + before,
        "归档段 {before} 条 + 没被取代的那段——「零回退损失」的可执行形态"
    );
    assert!(check_all(&all).is_empty(), "{:?}", check_all(&all));
}

/// 同一份快照**不落第二次**：判据是与完整史里最近一份快照逐 id 比对。
///
/// 这条钉的是幂等——同一轮里执行口可能被调多次（续写 / 答话），而压缩只发生一次。
#[tokio::test]
async fn the_same_consolidation_is_not_landed_twice() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    run_one_turn(&session, &sink).await;

    let store = EventWalStore::open(wal_path(&dir)).expect("open wal");
    let msgs = consolidated(&["snap-1"]);
    assert!(
        consolidate_into_wal(&store, &msgs, "session/main").is_some(),
        "第一次落格"
    );
    assert!(
        consolidate_into_wal(&store, &msgs, "session/main").is_none(),
        "同一份快照（逐 id 相同）不落第二次"
    );
    let n = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == crate::symbio_core::EVENT_SESSION_CONSOLIDATED)
        .count();
    assert_eq!(n, 1, "完整史里恰一条快照");
}

/// **不同**的一份快照（id 序列变了）⇒ 再落一条：套娃压缩（压了又压）由此成立。
///
/// 第二次落格会把**上一份快照**也归档（它落在被取代的前缀里）——所以两条判据分头看
/// 完整史（两份都在，审计可回溯）与现行窗口（只剩最新的那一份）。
#[tokio::test]
async fn a_newer_consolidation_lands_again() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    run_one_turn(&session, &sink).await;

    let store = EventWalStore::open(wal_path(&dir)).expect("open wal");
    assert!(consolidate_into_wal(&store, &consolidated(&["a"]), "session/main").is_some());
    assert!(
        consolidate_into_wal(&store, &consolidated(&["b"]), "session/main").is_some(),
        "id 序列不同 ⇒ 是新的一份，该落"
    );
    let is_snap = |e: &Event| e.kind == crate::symbio_core::EVENT_SESSION_CONSOLIDATED;
    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter().filter(|e| is_snap(e)).count(),
        2,
        "完整史里两份快照都在（审计 / 回溯可查）"
    );
    let window = current_window(&all);
    assert_eq!(
        window.iter().filter(|e| is_snap(e)).count(),
        1,
        "现行窗口只剩最新一份快照（上一份已被它取代、进了归档段）"
    );
    assert_eq!(window[0].payload["messages"][0]["id"], "b");
}

/// **轮外**压缩（用户重试 `retry_compaction`）走自己的一条路：那里没有「下一个
/// 执行口」可以寄存，所以自己开一次 WAL 落格——落格实现仍是同一个。
#[tokio::test]
async fn consolidate_outside_turn_lands_and_archives() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    run_one_turn(&session, &sink).await;

    consolidate_outside_turn(&session, &consolidated(&["snap-r"]), "session/main");

    let store = EventWalStore::open(wal_path(&dir)).expect("open wal");
    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter()
            .filter(|e| e.kind == crate::symbio_core::EVENT_SESSION_CONSOLIDATED)
            .count(),
        1,
        "轮外也落了一条快照"
    );
    assert_eq!(
        current_window(&all)[0].kind,
        crate::symbio_core::EVENT_SESSION_CONSOLIDATED
    );
    assert!(
        dir.join("transcripts").join("wal-archive").exists(),
        "轮外同样归档"
    );
}

/// **轮次水位**：归档把被取代的轮次移进了归档段，而轮号数的是**现行窗口**里的
/// `user.message` ⇒ 不补水位，压缩之后轮号会**从 1 重新数起**，而
/// `u-{turn}` 是幂等键。
///
/// 这条钉的就是「快照载荷必须带 `turns`」：去掉它，第二轮的 `user.message`
/// 会拿到 `turn = 0`（与归档段里的旧轮重号），事实网格里两轮撞成一轮。
#[tokio::test]
async fn the_turn_number_continues_across_archival() {
    let (session, dir, _tmp) = setup_full().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());

    run_one_turn(&session, &sink).await; // 第 1 轮：turn 0
    let store = EventWalStore::open(wal_path(&dir)).expect("open wal");
    assert!(
        consolidate_into_wal(&store, &consolidated(&["snap-1"]), "session/main").is_some(),
        "压掉第 1 轮"
    );
    let turns = current_window(&store.range(Seq::new(0)))[0].payload["turns"].as_u64();
    assert_eq!(turns, Some(1), "快照带走水位：它取代了 1 轮");
    drop(store);

    // 第二轮：现行窗口里一条 `user.message` 都没有（上一轮全被取代），靠水位才数得出 1。
    run_one_turn(&session, &sink).await;
    let store = EventWalStore::open(wal_path(&dir)).expect("open wal");
    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter().filter(|e| e.kind == EVENT_USER_MESSAGE).count(),
        2,
        "完整史看得见被归档的那一轮（口径归一后 `range(0)` 含前缀）"
    );
    let turn = current_window(&all)
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .map(|e| e.turn)
        .expect("第二轮的用户格应在现行窗口里");
    assert_eq!(turn, 1, "轮号接着水位走，不是从 0 重新数");
}
