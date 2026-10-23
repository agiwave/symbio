//! `plugin/run_actions.rs` 的单元测试：loop→session 内部动作面 `session/<sid>/_run/<op>`。
//!
//! 两半判据：
//! - **授权**：无令牌 / 令牌属于别的会话 / op 不在闭集或与地址不符 ⇒ `Forbidden`；
//!   非 `Action` 请求 ⇒ `Invalid`；
//! - **行为与帧形**：每个 op 的返回值是调用方经地址面能看到的全部，帧则经本会话
//!   订阅表观察（`Transcript::publish` 是消息帧的唯一出口）——两者一起锁住
//!   「写动作自己发帧」这条契约（[D-086](../../../../../docs/003-design/014-单环一表.md "D-086") §4）。

use super::*;
use crate::plugins::session::paths;
use crate::plugins::session::store::EventWalStore;
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::node_build::PresentationTree;
use crate::symbio_core::{
    vdfs_context, CapabilityMeta, EliminationDecider, Entity, Event, ExecAbortSignal, ExecEnv,
    ExecEventSink, ExecTranscriptWriter, ModelProvider, PluginError, PluginInvokeRequest,
    PluginSimpleRequest, Scope, Store as _, TurnOutput, TurnSettings, TurnSpec, VdfsChange,
    VdfsChangeSink, VdfsProvider, Verb, TURN_DISPATCH,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex as StdMutex};

const SID: &str = "sid";

/// 动作面不调模型：实现体只要满足 trait（构造 `TurnDispatch` 需要实例）。
struct NoopProvider;

#[async_trait]
impl ModelProvider for NoopProvider {
    fn provider_id(&self) -> &str {
        "run-actions-test"
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
        _system_prompt: &str,
        _messages: &[cm::ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError(
            "测试提供者不参与动作面".to_string(),
        ))
    }
}

struct NoopWriter;

#[async_trait]
impl ExecTranscriptWriter for NoopWriter {
    async fn apply(&self, _message: cm::ChatNode) {}
}

struct Fixture {
    _dir: tempfile::TempDir,
    plugin: SessionPlugin,
    session: Arc<PersistentChatSession>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let plugin = SessionPlugin::new(
        None,
        SessionConfig::default(),
        PluginDir::at(dir.path(), "session"),
    );
    // 临时会话的存储是实例级的——动作面必须复用同一个句柄，这里就是那个句柄。
    let session = Arc::new(PersistentChatSession::ephemeral(SID, plugin.config.clone()));
    Fixture {
        _dir: dir,
        plugin,
        session,
    }
}

fn turn_spec(session_id: &str, persistent: bool) -> TurnSpec {
    TurnSpec {
        session_id: session_id.to_string(),
        tool_archive_dir: std::env::temp_dir().join("tool-archives"),
        transcript_archive_dir: std::env::temp_dir().join("transcripts"),
        principal: "session/test".to_string(),
        scope: Scope::Root,
        persistent,
        context_window: 20,
        settings: TurnSettings {
            auto_compress: false,
            classify_enabled: false,
            compose_enabled: false,
            opening_enabled: false,
            progress_enabled: false,
            progress_interval_ms: 0,
            progress_min_rounds: 0,
            progress_max_per_turn: 0,
            presentation_tree: PresentationTree::Grouped,
            skill_compile_enabled: false,
            skill_fast_path: false,
            worker_force_prefix: String::new(),
            worker_keywords: String::new(),
            worker_min_chars: 0,
        },
    }
}

fn dispatch(session_id: &str, persistent: bool) -> Arc<TurnDispatch> {
    dispatch_as(session_id, persistent, "session/test")
}

fn dispatch_as(session_id: &str, persistent: bool, principal: &str) -> Arc<TurnDispatch> {
    let mut spec = turn_spec(session_id, persistent);
    spec.principal = principal.to_string();
    Arc::new(TurnDispatch {
        spec,
        facts: None,
        sink: ExecEventSink::silent(),
        writer: Arc::new(NoopWriter),
        abort: ExecAbortSignal::new(),
        provider: Arc::new(NoopProvider),
        parent: None,
        context_limit: 8192,
    })
}

/// 宿主 ctx：令牌与本轮会话句柄都挂在它上面（生产由编排入口设置）。
fn host(
    dispatch: Option<Arc<TurnDispatch>>,
    session: Option<&Arc<PersistentChatSession>>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    if let Some(d) = dispatch {
        req.set(TURN_DISPATCH, d);
    }
    if let Some(s) = session {
        req.set(SESSION_HANDLE, Arc::new(ChatSessionHandle::new(s.clone())));
    }
    Arc::new(req)
}

async fn call(
    plugin: &SessionPlugin,
    ctx: &VdfsContext,
    op: &str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    plugin
        .dispatch(
            ctx,
            &format!("{SID}/{}/{}", run_ops::SEG, op),
            VdfsRequest::Action {
                action: op.to_string(),
                payload,
            },
        )
        .await
}

/// 本会话订阅表上的全部变更（帧即 `VdfsChange.data`）。
fn subscribe(plugin: &SessionPlugin) -> Arc<StdMutex<Vec<VdfsChange>>> {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let sink: VdfsChangeSink = {
        let seen = seen.clone();
        Arc::new(move |change| seen.lock().unwrap().push(change))
    };
    plugin.change_subs.watch("", sink);
    seen
}

fn frames(seen: &Arc<StdMutex<Vec<VdfsChange>>>) -> Vec<VdfsChange> {
    seen.lock().unwrap().clone()
}

fn action(resp: VdfsResponse) -> VdfsActionResult {
    resp.into_action().expect("内部动作必须回 Action")
}

fn data(resp: VdfsResponse) -> Value {
    action(resp).data.expect("该动作应带数据")
}

fn unit(resp: VdfsResponse) {
    let result = action(resp);
    assert!(result.ok);
    assert!(result.data.is_none(), "该动作只回执，不携带数据");
}

fn msg(id: &str, role: cm::NodeRole, text: &str) -> cm::ChatNode {
    cm::ChatNode {
        id: id.to_string(),
        role: Some(role),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(cm::MessageContent::Text(text.to_string())),
        status: Some(cm::NodeStatus::Completed),
        ..Default::default()
    }
}

// ==================== 授权 ====================

/// 没有本轮令牌 ⇒ 拒绝。地址面对前端 / 工具 / 心跳都可达，只看地址等于给
/// 「改任意会话的写路径」开了第二个公开面。
#[tokio::test(flavor = "multi_thread")]
async fn run_requires_turn_dispatch_token() {
    let f = fixture();
    let ctx = vdfs_context(&host(None, Some(&f.session)));
    let err = call(&f.plugin, &ctx, run_ops::MESSAGES, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Forbidden(_)),
        "缺令牌必须 Forbidden，得到 {err:?}"
    );
}

/// 令牌只对**本轮会话**有效：拿 A 会话的令牌改 B 会话的地址 ⇒ 拒绝。
#[tokio::test(flavor = "multi_thread")]
async fn run_rejects_token_for_another_session() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch("other", false)), Some(&f.session)));
    let err = call(&f.plugin, &ctx, run_ops::MESSAGES, None)
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Forbidden(_)));
}

/// 动作名必须落在闭集内，且与地址末段字面相等——否则同一个地址能借任意动作名
/// 绕过「哪个 op 落在哪一行」的可读性。
#[tokio::test(flavor = "multi_thread")]
async fn run_rejects_unknown_op_and_address_mismatch() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    let err = call(&f.plugin, &ctx, "bogus", None).await.unwrap_err();
    assert!(matches!(err, VdfsError::Forbidden(_)), "不在闭集内");

    // 地址写 messages、动作名写 context-messages：两者必须逐字相等。
    let err = f
        .plugin
        .dispatch(
            &ctx,
            &format!("{SID}/{}/{}", run_ops::SEG, run_ops::MESSAGES),
            VdfsRequest::Action {
                action: run_ops::CONTEXT_MESSAGES.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Forbidden(_)), "地址与动作名不符");
}

/// `_run` 不是资源：读 / 写 / 列 / 删没有语义，只收 `Action`。
#[tokio::test(flavor = "multi_thread")]
async fn run_only_accepts_actions() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));
    let err = f
        .plugin
        .dispatch(
            &ctx,
            &format!("{SID}/{}/{}", run_ops::SEG, run_ops::MESSAGES),
            VdfsRequest::Stat,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Invalid(_)), "得到 {err:?}");
}

// ==================== 行表（闭集的组织方式，D-094）====================

/// 闭集 = 行表自身：**op 词表的每个常量在行表里恰好出现一次**。
///
/// 这条判据替代的正是旧形态里「core 的手写 `ALL` 与 `match` 臂人工对账」那件事。
/// 两个方向都响：加常量不加行 ⇒ 红（授权会 `Forbidden`，而调用方以为它在闭集里）；
/// 加行不改本判据 ⇒ 红（实现了一个词表没说的行为，两侧从此对不上）。
/// 期望集在测试里逐一点名——钉「表与词表一致」正是测试的份内事。
#[test]
fn table_covers_every_run_op_constant() {
    let expected = [
        run_ops::MESSAGES,
        run_ops::CONTEXT_MESSAGES,
        run_ops::ADD_BY_RUNNER,
        run_ops::ADD_HERE,
        run_ops::EDIT_NODES,
        run_ops::REMOVE_NODES,
        run_ops::REPLACE_ALL,
        run_ops::DRAIN_PARKED,
        run_ops::CONSOLIDATE,
        run_ops::TAKE_SUPPLEMENTS,
        run_ops::WORKER_PROGRESS,
        run_ops::RECALL_SNAPSHOTS,
        run_ops::ELIMINATION_REVOKED,
        run_ops::COMPRESSION_BEGIN,
        run_ops::COMPRESSION_FINISH,
    ];
    let mut table: Vec<&str> = RUN_OPS.iter().map(|r| r.op).collect();
    table.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(
        table, expected,
        "行表必须与 op 词表逐一对应：加常量要加行，加行要改本判据"
    );
}

/// 行表无重号：一个 op 两行 = 两个处理函数抢同一个地址，行为取决于 `find`
/// 的先后——那是静默的。装配期就能查，不必等运行期。
#[test]
fn table_has_no_duplicate_ops() {
    let mut seen = std::collections::HashSet::new();
    for row in RUN_OPS {
        assert!(
            seen.insert(row.op),
            "行表里有重复的 op：{}（一个 op 只能有一个处理函数）",
            row.op
        );
    }
}

/// 临时会话没有会话句柄 ⇒ Internal：**不许按 id 重建**（那会拿到另一份空存储）。
#[tokio::test(flavor = "multi_thread")]
async fn run_ephemeral_requires_session_handle() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), None));
    let err = call(&f.plugin, &ctx, run_ops::MESSAGES, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Internal(_)),
        "缺句柄的临时会话必须 Internal，得到 {err:?}"
    );
}

// ==================== 消息读写 ====================

/// `add-by-runner` / `add-here` 落库并回权威副本；`messages` / `context-messages`
/// 读回同一批——地址面一次往返，号从存储换进调用方。
#[tokio::test(flavor = "multi_thread")]
async fn run_add_then_read_round_trip() {
    let f = fixture();
    let seen = subscribe(&f.plugin);
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    let first = msg("m1", cm::NodeRole::User, "你好");
    let persisted = data(
        call(
            &f.plugin,
            &ctx,
            run_ops::ADD_BY_RUNNER,
            Some(json!({ "messages": [first] })),
        )
        .await
        .unwrap(),
    );
    let persisted: Vec<cm::ChatNode> = serde_json::from_value(persisted).unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, "m1");
    assert_eq!(persisted[0].content.as_ref().unwrap().to_text(), "你好");
    let first_seq = persisted[0].seq.expect("存储必须分配权威 seq");
    assert!(
        first_seq < crate::plugins::session::transcript::InflightSeq::BASE,
        "回给调用方的必须是存储号，不是在途号"
    );

    // 写动作自己发帧：一条消息一次变更，载荷是权威副本。
    let published = frames(&seen);
    assert_eq!(published.len(), 1, "一条消息一次帧");
    assert_eq!(published[0].path, format!("{SID}/message/m1"));
    assert_eq!(
        published[0].data.as_ref().unwrap()["seq"],
        json!(first_seq),
        "帧里带的必须是存储号"
    );

    // 第二条走 `add-here`：同一条落库出口，号继续单调。
    let persisted = data(
        call(
            &f.plugin,
            &ctx,
            run_ops::ADD_HERE,
            Some(json!({ "messages": [msg("m2", cm::NodeRole::Assistant, "好的")] })),
        )
        .await
        .unwrap(),
    );
    let persisted: Vec<cm::ChatNode> = serde_json::from_value(persisted).unwrap();
    assert!(persisted[0].seq.unwrap() > first_seq);

    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m1", "m2"]
    );

    let context: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::CONTEXT_MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(context.len(), 2, "上下文候选是同一批消息");
}

/// `edit-nodes` 的帧形由调用方给：`state` 只发身份 + 状态（正文早已由 delta 上线），
/// `message` 发完整正文（首次上线）。两者压成一个形状，必有一处静默回退旧值。
#[tokio::test(flavor = "multi_thread")]
async fn run_edit_nodes_honours_frame_shape() {
    let f = fixture();
    let seen = subscribe(&f.plugin);
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    data(
        call(
            &f.plugin,
            &ctx,
            run_ops::ADD_BY_RUNNER,
            Some(json!({ "messages": [msg("m1", cm::NodeRole::Assistant, "原文")] })),
        )
        .await
        .unwrap(),
    );
    seen.lock().unwrap().clear();

    // 状态帧：正文被剥掉。
    let edited = msg("m1", cm::NodeRole::Assistant, "改后");
    let returned = data(
        call(
            &f.plugin,
            &ctx,
            run_ops::EDIT_NODES,
            Some(json!({ "nodes": [edited], "frame": "state" })),
        )
        .await
        .unwrap(),
    );
    let returned: Vec<cm::ChatNode> = serde_json::from_value(returned).unwrap();
    assert_eq!(returned[0].content.as_ref().unwrap().to_text(), "改后");

    let state_frames = frames(&seen);
    assert_eq!(state_frames.len(), 1);
    let frame_data = state_frames[0].data.as_ref().unwrap();
    assert_eq!(frame_data["id"], "m1");
    assert_eq!(frame_data["status"], "completed");
    assert!(frame_data["content"].is_null(), "状态帧不得携带正文");

    // 完整消息帧：正文整条替换。
    seen.lock().unwrap().clear();
    let edited = msg("m1", cm::NodeRole::Assistant, "再改");
    data(
        call(
            &f.plugin,
            &ctx,
            run_ops::EDIT_NODES,
            Some(json!({ "nodes": [edited], "frame": "message" })),
        )
        .await
        .unwrap(),
    );
    let message_frames = frames(&seen);
    assert_eq!(message_frames.len(), 1);
    assert_eq!(message_frames[0].data.as_ref().unwrap()["content"], "再改");

    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(listed[0].content.as_ref().unwrap().to_text(), "再改");
}

/// `remove-nodes`：删除就是一次状态迁移——存储里去掉，实时面发 `removed` 帧。
#[tokio::test(flavor = "multi_thread")]
async fn run_remove_nodes_broadcasts_removal() {
    let f = fixture();
    let seen = subscribe(&f.plugin);
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    for (id, role) in [("m1", cm::NodeRole::User), ("m2", cm::NodeRole::Assistant)] {
        data(
            call(
                &f.plugin,
                &ctx,
                run_ops::ADD_BY_RUNNER,
                Some(json!({ "messages": [msg(id, role, id)] })),
            )
            .await
            .unwrap(),
        );
    }
    seen.lock().unwrap().clear();

    unit(
        call(
            &f.plugin,
            &ctx,
            run_ops::REMOVE_NODES,
            Some(json!({ "ids": ["m1"] })),
        )
        .await
        .unwrap(),
    );

    let removal = frames(&seen);
    assert_eq!(removal.len(), 1);
    assert_eq!(removal[0].path, format!("{SID}/message/m1"));
    assert_eq!(removal[0].data.as_ref().unwrap()["status"], "removed");

    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m2"]
    );
}

/// `replace-all`：整表重写后逐条发删除帧、新首条发完整消息帧——前端因此
/// 不会停在压缩前的列表上。
#[tokio::test(flavor = "multi_thread")]
async fn run_replace_all_broadcasts_rewrite() {
    let f = fixture();
    let seen = subscribe(&f.plugin);
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    for id in ["m1", "m2"] {
        data(
            call(
                &f.plugin,
                &ctx,
                run_ops::ADD_BY_RUNNER,
                Some(json!({ "messages": [msg(id, cm::NodeRole::User, id)] })),
            )
            .await
            .unwrap(),
        );
    }
    seen.lock().unwrap().clear();

    let head = msg("snapshot", cm::NodeRole::Assistant, "压缩后的摘要");
    unit(
        call(
            &f.plugin,
            &ctx,
            run_ops::REPLACE_ALL,
            Some(json!({ "nodes": [head], "dropped": ["m1", "m2"], "head": head })),
        )
        .await
        .unwrap(),
    );

    let emitted = frames(&seen);
    assert_eq!(emitted.len(), 3, "两条删除帧 + 一条新首条");
    assert_eq!(emitted[0].data.as_ref().unwrap()["status"], "removed");
    assert_eq!(emitted[1].data.as_ref().unwrap()["status"], "removed");
    assert_eq!(emitted[2].path, format!("{SID}/message/snapshot"));
    assert_eq!(emitted[2].data.as_ref().unwrap()["content"], "压缩后的摘要");

    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["snapshot"]
    );
}

// ==================== 无事实源时的空操作 ====================

/// 临时会话没有事实源：补落寄存 / 压缩事实都无事可做，但**不是错误**。
#[tokio::test(flavor = "multi_thread")]
async fn run_fact_writes_are_noops_without_store() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    unit(
        call(&f.plugin, &ctx, run_ops::DRAIN_PARKED, None)
            .await
            .unwrap(),
    );
    unit(
        call(
            &f.plugin,
            &ctx,
            run_ops::CONSOLIDATE,
            Some(json!({ "messages": [] })),
        )
        .await
        .unwrap(),
    );
}

/// 补充队列为空 ⇒ `null`；子会话进展 / 快照召回为空 ⇒ `[]`；撤销闸 ⇒ `false`。
/// 值形状是 loop 侧的解析依据，不能变成「缺字段」。
#[tokio::test(flavor = "multi_thread")]
async fn run_queries_return_declared_shapes() {
    let f = fixture();
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    let batch = data(
        call(&f.plugin, &ctx, run_ops::TAKE_SUPPLEMENTS, None)
            .await
            .unwrap(),
    );
    assert!(batch.is_null(), "空补充队列是 null，不是缺键");

    let progress = data(
        call(&f.plugin, &ctx, run_ops::WORKER_PROGRESS, None)
            .await
            .unwrap(),
    );
    assert_eq!(progress, json!([]));

    let snapshots = data(
        call(&f.plugin, &ctx, run_ops::RECALL_SNAPSHOTS, None)
            .await
            .unwrap(),
    );
    assert_eq!(snapshots, json!([]));

    let revoked = data(
        call(&f.plugin, &ctx, run_ops::ELIMINATION_REVOKED, None)
            .await
            .unwrap(),
    );
    assert_eq!(revoked, json!(false));
}

// ==================== 压缩节点 ====================

/// `compression-begin` 发布占位并回执「这次要不要跑」（熔断判定在会话侧）；
/// `compression-finish` 发布终态**并落库**，回给 loop 的是存储的权威副本（带号）。
#[tokio::test(flavor = "multi_thread")]
async fn run_compression_begin_and_finish() {
    let f = fixture();
    let seen = subscribe(&f.plugin);
    let ctx = vdfs_context(&host(Some(dispatch(SID, false)), Some(&f.session)));

    let begin_reply = data(
        call(
            &f.plugin,
            &ctx,
            run_ops::COMPRESSION_BEGIN,
            Some(json!({ "node_id": "c1" })),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        begin_reply["skipped"], false,
        "熔断未触发 ⇒ loop 应照常压缩（D-086 §6）"
    );
    let begin = frames(&seen);
    assert_eq!(begin.len(), 1);
    assert_eq!(begin[0].data.as_ref().unwrap()["status"], "streaming");
    assert_eq!(
        begin[0].data.as_ref().unwrap()["content"],
        "正在压缩上下文…"
    );

    seen.lock().unwrap().clear();
    let node = data(
        call(
            &f.plugin,
            &ctx,
            run_ops::COMPRESSION_FINISH,
            Some(json!({
                "node_id": "c1",
                "status": "completed",
                "text": "已压缩 6 → 2 条",
                "stats": { "before": 6, "after": 2 },
            })),
        )
        .await
        .unwrap(),
    );
    let node: cm::ChatNode = serde_json::from_value(node).unwrap();
    assert_eq!(node.id, "c1");
    assert_eq!(node.status, Some(cm::NodeStatus::Completed));
    assert_eq!(node.content.as_ref().unwrap().to_text(), "已压缩 6 → 2 条");
    assert!(node.seq.is_some(), "落库回执必须是存储的权威副本");
    assert_eq!(node.meta.as_ref().unwrap()["before"], 6, "stats 并入 meta");

    // 落库后刷新仍能看到这条记录（否则历史「突然变短」却没有交代）。
    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["c1"]
    );
}

// ==================== 持久会话的缺句柄回退 ====================

/// 持久会话的引擎无实例状态：缺句柄时可经 `Convergence` 按 id 重建（临时会话
/// 不适用，见上一条）。重建读的是同一份存储。
#[tokio::test(flavor = "multi_thread")]
async fn run_persistent_rebuilds_from_store_without_handle() {
    let f = fixture();
    // 先落元数据（`session.json` 是会话存在的判据），再走引擎写 WAL——
    // D-049：`v2-events.wal` 是消息的唯一事实源，`messages.json` 停写。
    f.plugin.save_session(&Session::new(SID)).await.unwrap();
    let engine = f.plugin.open_chat_session(SID).await.unwrap();
    // 变更事实需要一条溯源锚（「这次变更站在哪份状态上」）；真实轮次里它由
    // 本轮开轮事件提供，这里补一条**非消息**事实——它只提供号，不会被重建
    // 折成一条消息节点。名字取表上的 `system.triggered`（`system × opened`）：
    // 手搓一个不在名字表上的名字会被写侧闸门判 `OffGrid`（ 段四），
    // 而「测试里编一个名字」正是那道闸要拦的形状。
    engine
        .facts()
        .unwrap()
        .expect("持久会话有事实源")
        .append(Event::pending(
            "anchor",
            crate::symbio_core::EVENT_SYSTEM_TRIGGERED,
            Entity::System,
            Verb::Opened,
            0,
            "user",
        ))
        .unwrap();
    engine
        .apply(MessageChange::Born {
            nodes: vec![msg("m1", cm::NodeRole::User, "盘上的消息")],
            // 承载事实由运行器落的节点不写 `message.added`；测试没有运行器，
            // 只能走 `Here` 让事实真进 WAL（WAL 是消息的唯一真源）。
            carrier: Carrier::Here,
        })
        .await
        .unwrap();

    let ctx = vdfs_context(&host(Some(dispatch(SID, true)), None));
    let listed: Vec<cm::ChatNode> = serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::MESSAGES, None)
            .await
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].content.as_ref().unwrap().to_text(), "盘上的消息");
}

// ==================== 部署级淘汰覆盖层 ====================
//
// 扫描范围跟着存储布局走，两半都在 session：`deep_snapshots` 深遍全会话（含
// 嵌套），折叠判据在 core，动作只交结论值（D-086 §4）。loop 侧只钉值搬运
// （`loop/tools/tool_executor/elimination.test.rs`）。

/// 在 `<root>/<safe(sid)>/v2-events.wal` 落一条事件（平级会话布局）。
fn append_to_session(root: &std::path::Path, session_id: &str, event: Event) {
    let dir = paths::session_dir(root, session_id);
    std::fs::create_dir_all(&dir).expect("建会话目录");
    EventWalStore::open(dir.join(paths::V2_WAL_FILE))
        .expect("打开 WAL")
        .append(event)
        .expect("入格");
}

/// 在 `<root>/<safe(父)>/sessions/<safe(子)>/v2-events.wal` 落一条事件
/// （嵌套子会话布局：子会话住在父目录的 `sessions/` 里）。
fn append_to_sub_session(root: &std::path::Path, parent_sid: &str, child_sid: &str, event: Event) {
    let dir = root
        .join(paths::safe_id(parent_sid))
        .join(paths::SUB_SESSIONS_SUBDIR)
        .join(paths::safe_id(child_sid));
    std::fs::create_dir_all(&dir).expect("建子会话目录");
    EventWalStore::open(dir.join(paths::V2_WAL_FILE))
        .expect("打开子会话 WAL")
        .append(event)
        .expect("入格");
}

/// 以某主体查一次撤销闸（持久会话按 id 从磁盘重建——扫描要看整份部署）。
async fn revoked(f: &Fixture, principal: &str) -> bool {
    let ctx = vdfs_context(&host(Some(dispatch_as(SID, true, principal)), None));
    serde_json::from_value(data(
        call(&f.plugin, &ctx, run_ops::ELIMINATION_REVOKED, None)
            .await
            .unwrap(),
    ))
    .unwrap()
}

/// 裁决落在**另一个会话**的 WAL 里，本会话照样被收回。
///
/// 「换个会话就能绕开淘汰」是本闸要消灭的静默面——按会话扫覆盖层就会得到它。
#[tokio::test(flavor = "multi_thread")]
async fn run_elimination_revoked_sees_a_verdict_from_another_session() {
    let f = fixture();
    append_to_session(
        &f.plugin.storage_dir(),
        "sess-a",
        EliminationDecider.elimination_event(crate::symbio_core::PRINCIPAL_MAIN, -1, 7),
    );

    assert!(revoked(&f, crate::symbio_core::PRINCIPAL_MAIN).await);
}

/// **嵌套子会话**里的裁决在本会话生效（ 批发现并修的静默面）。
///
/// 子会话不是顶层平级目录，只扫一层的覆盖层整棵子树都看不见。
#[tokio::test(flavor = "multi_thread")]
async fn run_elimination_revoked_scans_nested_sub_sessions() {
    let f = fixture();
    append_to_sub_session(
        &f.plugin.storage_dir(),
        "sess-parent",
        "sess-child",
        EliminationDecider.elimination_event(crate::symbio_core::PRINCIPAL_MAIN, -1, 7),
    );

    assert!(revoked(&f, crate::symbio_core::PRINCIPAL_MAIN).await);
}

/// 恢复可逆：revoke + restore（last-wins）⇒ 闸门放行。
///
/// 「可申诉」若不是机制而是文档话，这条用例就红。
#[tokio::test(flavor = "multi_thread")]
async fn run_elimination_revoked_is_flipped_back_by_a_restoration() {
    let f = fixture();
    let root = f.plugin.storage_dir();
    let principal = crate::symbio_core::PRINCIPAL_MAIN;
    append_to_session(
        &root,
        "sess-a",
        EliminationDecider.elimination_event(principal, -1, 7),
    );
    append_to_session(
        &root,
        "sess-a",
        EliminationDecider.restoration_event(principal, "申诉受理：上游延迟有凭证", 9),
    );

    assert!(!revoked(&f, principal).await);
}

/// 按主体精确命中：淘汰的是 `agent:ghost`，本主体（main）不受殃及、ghost 自己被拒。
#[tokio::test(flavor = "multi_thread")]
async fn run_elimination_revoked_hits_only_the_verdict_subject() {
    let f = fixture();
    append_to_session(
        &f.plugin.storage_dir(),
        "sess-a",
        EliminationDecider.elimination_event("session/ghost", -1, 7),
    );

    assert!(!revoked(&f, crate::symbio_core::PRINCIPAL_MAIN).await);
    assert!(revoked(&f, "session/ghost").await);
}

/// 全新部署（还没有任何会话事实源）⇒ 空覆盖层，不拒也不炸。
#[tokio::test(flavor = "multi_thread")]
async fn run_elimination_revoked_on_an_empty_deployment_is_false() {
    let f = fixture();
    assert!(!revoked(&f, crate::symbio_core::PRINCIPAL_MAIN).await);
}
