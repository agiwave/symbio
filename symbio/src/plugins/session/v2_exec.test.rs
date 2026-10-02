//! v2 执行器验收——full 档无工具轮的三条硬契约：
//!
//! 1. **事实原生入格**：用户格 + final 格落在本会话的 v2 WAL，不变量全绿
//!    （与 bridge 档同一份网格口径——切换开关只换写法，不换网格）；
//! 2. **流式回同一出口**：生成增量经 UiBridge 变成 v1 的 UI 帧（首片快照
//!    建节点 + 后续窄帧），帧语义与 model 插件直发同构；
//! 3. **兜底也是一句话**（I3）：生成失败落 fallback 格后上抛——用户侧
//!    失败呈现走 v1 的 Failed 出口，网格里不缺格。

use std::sync::{Arc, Mutex};

use super::*;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::{
    check_all, CapabilityMeta, ExecTranscriptWriter, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL,
};

use super::super::config::{SessionConfig, V2Mode};
use super::super::store::SessionStore;
use super::super::types::Session;

/// 帧收集口（测试的 ExecEventSink 出口）。
struct CollectingFrames(Mutex<Vec<cm::ChatMessage>>);

#[async_trait::async_trait]
impl ExecTranscriptWriter for CollectingFrames {
    async fn apply(&self, m: cm::ChatMessage) {
        self.0.lock().unwrap().push(m);
    }
}

/// 忠实假 provider：像真实 SSE 一样经 `env` 发「快照 + 窄帧」，返回聚合全文。
/// `fail` 为真时按生成失败返回（演练 I3 兜底格）。
struct FaithfulProvider {
    fail: bool,
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
        _messages: &[cm::ChatMessage],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        env: &crate::symbio_core::ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        if self.fail {
            return Err(PluginError::InternalError("模型断了".into()));
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

    let out = execute_tool_free_turn(ToolFreeTurn {
        session: &session,
        provider: Arc::new(FaithfulProvider { fail: false }),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "你好",
        window_turns: 6,
    })
    .await
    .expect("full 档执行成功");

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

    let err = execute_tool_free_turn(ToolFreeTurn {
        session: &session,
        provider: Arc::new(FaithfulProvider { fail: true }),
        system_prompt: "system",
        abort: crate::symbio_core::ExecAbortSignal::new(),
        root_id: "r-1",
        sink: &sink,
        user_text: "你好",
        window_turns: 6,
    })
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
