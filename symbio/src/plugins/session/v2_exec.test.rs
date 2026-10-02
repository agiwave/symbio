//! v2 执行器验收——full 档无工具轮的三条硬契约：
//!
//! 1. **事实原生入格**：用户格 + final 格落在本会话的 v2 WAL，不变量全绿
//!    （与 bridge 档同一份网格口径——切换开关只换写法，不换网格）；
//! 2. **流式回同一出口**：生成增量经 UiBridge 变成 v1 的 UI 帧（首片快照
//!    建节点 + 后续窄帧），帧语义与 model 插件直发同构；
//! 3. **兜底也是一句话**（I3）：生成失败落 fallback 格后上抛——用户侧
//!    失败呈现走 v1 的 Failed 出口，网格里不缺格；
//! 4. **中止不是失败**：生成被中止时**不落收束格**（网格少一格是诚实缺口，
//!    ADR-045 同源），上抛 `Aborted` 走独立出口——不呈现「失败」也不进兜底率
//!    分母。中止与失败的差别在**类型上**可见，不靠错误文本猜。

use std::sync::{Arc, Mutex};

use super::*;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::{
    check_all, CapabilityMeta, ExecTranscriptWriter, PluginSimpleRequest, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL,
};

use super::super::config::{SessionConfig, V2Mode};
use super::super::store::SessionStore;
use super::super::types::Session;

/// 测试请求上下文（工具轮才需要能力注册表；无工具轮一个空信封足够）。
fn test_ctx() -> Arc<dyn crate::symbio_core::PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 无工具轮的入参包（本文件的三个用例都是这一形态）。
fn tool_free_req<'a>(
    session: &'a PersistentChatSession,
    provider: Arc<dyn ModelProvider>,
    sink: &'a crate::symbio_core::ExecEventSink,
    abort: crate::symbio_core::ExecAbortSignal,
) -> V2Turn<'a> {
    V2Turn {
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
        window_turns: 6,
        tools: &[],
    }
}

/// 帧收集口（测试的 ExecEventSink 出口）。
struct CollectingFrames(Mutex<Vec<cm::ChatMessage>>);

#[async_trait::async_trait]
impl ExecTranscriptWriter for CollectingFrames {
    async fn apply(&self, m: cm::ChatMessage) {
        self.0.lock().unwrap().push(m);
    }
}

/// 假 provider 的行为档：忠实流式 / 生成失败 / 中止 / 工具轮。
#[derive(Clone, Copy)]
enum Behavior {
    /// 像真实 SSE 一样经 `env` 发「快照 + 窄帧」，返回聚合全文。
    Faithful,
    /// 按生成失败返回（演练 I3 兜底格）。
    Fail,
    /// 按中止返回（演练「中止不落收束格」）。
    Abort,
    /// 首次请求回一个工具调用（附中途正文），第二次回正文——演练工具轮。
    ToolThenAnswer,
}

/// 忠实假 provider：行为由 [`Behavior`] 选定（同一传输形状，只换结局）。
struct FaithfulProvider {
    behavior: Behavior,
    /// 每次请求收到的 prompt（工具轮的验收要回读**下一次请求**）。
    prompts: Arc<Mutex<Vec<String>>>,
}

impl FaithfulProvider {
    fn new(behavior: Behavior) -> (Self, Arc<Mutex<Vec<String>>>) {
        let prompts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
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
        messages: &[cm::ChatMessage],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        env: &crate::symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        // 记下本次请求的 prompt——工具轮的硬契约是「工具结果进了下一次请求」，
        // 那条断言只能回读请求体（不变量/网格都证明不了）。
        let prompt = messages
            .first()
            .and_then(|m| m.content.as_ref())
            .map(|c| c.to_text())
            .unwrap_or_default();
        let round = {
            let mut p = self.prompts.lock().unwrap();
            p.push(prompt);
            p.len()
        };
        match self.behavior {
            Behavior::Fail => return Err(PluginError::InternalError("模型断了".into())),
            Behavior::Abort => return Err(PluginError::Aborted),
            Behavior::Faithful => {}
            Behavior::ToolThenAnswer => {
                if round == 1 {
                    // 中途正文：真实模型调工具前常先说一句。它必须被定格并切节点，
                    // 否则会与收尾正文粘成一条。
                    env.sink()
                        .emit(cm::ChatMessage {
                            id: "n-tool".into(),
                            parent_id: Some("r-1".into()),
                            role: Some(cm::MessageRole::Assistant),
                            msg_type: Some(cm::MessageType::Text),
                            content: Some(cm::MessageContent::Text("我先查一下。".into())),
                            status: Some(cm::MessageStatus::Streaming),
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
                        }],
                        response_text_child_id: "n-tool".into(),
                        ..Default::default()
                    });
                }
            }
        }
        // 真实链路的帧形态：正文快照（Text × Streaming）先行，窄帧随后。
        env.sink()
            .emit(cm::ChatMessage {
                id: "n-1".into(),
                parent_id: Some("r-1".into()),
                role: Some(cm::MessageRole::Assistant),
                msg_type: Some(cm::MessageType::Text),
                content: Some(cm::MessageContent::Text("秋".into())),
                status: Some(cm::MessageStatus::Streaming),
                ..Default::default()
            })
            .await;
        env.sink()
            .emit(cm::ChatMessage {
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

/// 持久会话 + Full 档装配；返回 (会话, 会话目录, 清理句柄)。
async fn setup_full() -> (PersistentChatSession, std::path::PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "v2_exec_test".to_string();
    // 先落盘一次空会话，使 session_dir 能解析到实际目录（v2 WAL 的安放处）。
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store.session_dir(&session_id).expect("应返回会话目录");
    let config = SessionConfig {
        v2_mode: V2Mode::Full,
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
    .expect("full 档执行成功");
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
    assert_eq!(got[0].status, Some(cm::MessageStatus::Streaming));
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
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 2, "用户格 + final 格：{snap:?}");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert_eq!(snap[0].turn, 0, "turn 号 = WAL 内 user.message 计数");
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FINAL);
    assert_eq!(snap[1].payload["text"], "秋天好", "final 落的是聚合全文");
    assert!(snap[1].produced_by.is_some(), "final 溯源指向用户格");
}

/// 失败轮（I3）：fallback 格照落（网格不缺格），错误上抛给 chat_loop 走
/// Failed 出口——用户侧失败呈现与 v1 一致。
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
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 2, "用户格 + fallback 格：{snap:?}");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
    assert_eq!(snap[1].kind, EVENT_ASSISTANT_FALLBACK);
    let why = snap[1].payload["why"].as_str().unwrap_or_default();
    assert!(why.contains("模型断了"), "兜底格要记失败原因：{why}");
}

/// 中止轮：**不落收束格**（网格只剩用户格），上抛 `Aborted` 走独立出口——
/// 用户自己按的停止不该有错误条与重试入口，也不该进兜底率分母。
#[tokio::test]
async fn tool_free_turn_abort_leaves_only_user_event_and_bubbles_aborted() {
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

    assert!(
        matches!(err, PluginError::Aborted),
        "上抛的是中止本体，不被压成失败：{err:?}"
    );

    // 网格里**只有用户格**——中止不是失败，不落兜底格；少一格是诚实缺口。
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 1, "只剩用户格（中止不落收束格）：{snap:?}");
    assert_eq!(snap[0].kind, EVENT_USER_MESSAGE);
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
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

    let res = execute_turn(V2Turn {
        session: &session,
        provider: Arc::new(provider),
        // 无插件宿主 ⇒ 工具分发按「没有父插件」诚实失败——本用例验的是**链路**
        // （落格 / 帧 / 回灌），工具真正跑起来由 e2e 覆盖。失败是信息性的：
        // 结果照样回灌，循环照常继续（正是 v1 的口径）。
        parent: None,
        session_dir: super::super::test_dir(),
        ctx: test_ctx(),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "读一下 a.md",
        window_turns: 6,
        tools: &[CapabilityMeta {
            name: "vdfs_read".into(),
            description: "读文件".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            keywords: vec![],
            category: None,
            examples: None,
            context_retention: None,
        }],
    })
    .await
    .expect("工具轮执行成功");
    let out = res.output;
    assert_eq!(out.text, "秋天好", "收尾轮的正文");

    // ── 回读请求体：工具结果进了下一次请求 ────────────────────────────────
    let seen = prompts.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "工具轮 + 收尾轮 = 两次请求：{seen:?}");
    assert!(
        seen[0].contains("读一下 a.md"),
        "第一次请求带本轮用户发言：{}",
        seen[0]
    );
    assert!(
        !seen[0].contains("工具结果"),
        "第一次请求还没有工具结果可言：{}",
        seen[0]
    );
    assert!(
        seen[1].contains("vdfs_read"),
        "第二次请求要带上模型请求过的工具名：{}",
        seen[1]
    );
    assert!(
        seen[1].contains("No parent plugin"),
        "第二次请求要带上工具结果正文（含失败——失败也是信息）：{}",
        seen[1]
    );

    // ── 落库消息：助手中途正文 + ToolCall + 工具结果 ──────────────────────
    assert_eq!(
        res.messages.len(),
        3,
        "正文 + 工具调用 + 工具结果：{:?}",
        res.messages
    );
    assert_eq!(res.messages[0].msg_type, Some(cm::MessageType::Text));
    assert_eq!(res.messages[0].status, Some(cm::MessageStatus::Completed));
    assert_eq!(res.messages[0].parent_id.as_deref(), Some("r-1"));
    let call = &res.messages[1];
    assert_eq!(call.msg_type, Some(cm::MessageType::ToolCall));
    assert_eq!(call.id, "tc-1", "工具节点 id 取 provider 给的节点 id");
    assert_eq!(
        call.tool_call_id.as_deref(),
        Some("call_1"),
        "wire id 原样保留"
    );
    let tool_result = &res.messages[2];
    assert_eq!(tool_result.parent_id.as_deref(), Some("tc-1"));
    assert_eq!(tool_result.role, Some(cm::MessageRole::Tool));

    // ── UI 帧：中途正文被**定格**（不是永远流式中），工具节点与结果都在 ────
    let got = frames.0.lock().unwrap();
    let text_frames: Vec<_> = got
        .iter()
        .filter(|m| {
            m.msg_type == Some(cm::MessageType::Text) && m.role == Some(cm::MessageRole::Assistant)
        })
        .collect();
    assert!(
        text_frames
            .iter()
            .any(|m| m.status == Some(cm::MessageStatus::Completed)),
        "中途正文必须被定格：{got:?}"
    );
    assert!(
        got.iter()
            .any(|m| m.msg_type == Some(cm::MessageType::ToolCall) && m.id == "tc-1"),
        "工具调用节点必须广播给前端：{got:?}"
    );
    assert!(
        got.iter()
            .any(|m| m.role == Some(cm::MessageRole::Tool)
                && matches!(&m.content, Some(cm::MessageContent::Text(t)) if t.contains("No parent plugin"))),
        "工具结果节点必须广播给前端：{got:?}"
    );

    // ── 网格：用户格 + 产物格 + final 格，产物格溯源指向本轮用户格 ────────
    let store = EventWalStore::open(dir.join("v2-events.wal")).expect("open wal");
    let snap = store.range(Seq::new(0));
    assert_eq!(snap.len(), 3, "用户格 + 产物格 + final 格：{snap:?}");
    assert!(check_all(&snap).is_empty(), "{:?}", check_all(&snap));
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
