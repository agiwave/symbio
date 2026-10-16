//! `resume` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `resume.rs` 只保留生产代码，测试全部放本文件。

use super::*;

fn tc(id: &str, status: NodeStatus) -> ChatNode {
    ChatNode {
        id: id.into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        status: Some(status),
        meta: Some(json!({ "style_tag": "tool", "started_at": 1_700_000_000_000i64 })),
        ..Default::default()
    }
}

/// 中止收口：父 ToolCall 必须从 `Streaming` 落地为终态。
///
/// 回归：`process_tool_resume_action` 在重跑工具**之前**就把父节点广播成
/// `Streaming`（有意为之——否则用户点「批准执行」后画面毫无变化；那是广播帧，
/// 持久层拒绝瞬态落盘），而中止路径不得直接 `return Ok(Done)`。那个节点既不是
/// 流式节点、也不会再被任何后续流程碰，于是前端**永远**显示"运行中"
/// （违反 `docs/018-node-state-streaming.md` §8 #11）。
#[test]
fn aborted_resume_finalizes_parent_tool_call() {
    let mut messages = vec![
        tc("tc1", NodeStatus::Streaming),
        ChatNode {
            id: "child".into(),
            parent_id: Some("tc1".into()),
            ..Default::default()
        },
    ];

    // 返回值是**完整快照**（广播用），与就地定稿的存储副本同一形态
    let snapshot = finalize_aborted_parent(&mut messages, 0, "tc1");

    assert_eq!(
        messages[0].status,
        Some(NodeStatus::Completed),
        "父节点不得停在 Streaming：它是已落库节点，没有人会再来收口"
    );
    assert_eq!(snapshot.id, "tc1", "完整快照必须保留节点 id");
    assert_eq!(snapshot.status, Some(NodeStatus::Completed));
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("failure_kind")),
        Some(&json!("aborted")),
        "成因必须可区分：「被中止」与「批次没轮到」「被钩子拦下」不是同一件事"
    );
    assert!(
        snapshot.error.is_none(),
        "中止不是执行失败，不得在父节点挂 error —— 那会让前端多渲染一份错误文本"
    );
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("success")),
        Some(&json!(false)),
        "未完成必须如实报告 success=false（前端据此不渲染成功态）"
    );
    assert_eq!(
        snapshot.meta.as_ref().and_then(|m| m.get("started_at")),
        Some(&json!(1_700_000_000_000i64)),
        "meta 合并不得丢弃既有键（完整快照 = 权威副本 + 终态，不是只剩终态字段）"
    );
    // 子节点不受影响：收口只针对父节点
    assert_eq!(messages[1].status, None);
}

/// 索引越界即程序错误：`tc_idx` 由步骤 2 在同一份 `messages` 上定位得出，
/// 二者不可能失配——失配说明定位逻辑本身坏了，当场 panic 暴露问题
/// （而不是造一个只有 id 的假快照掩盖过去）。
#[test]
#[should_panic(expected = "步骤 2 已校验过 tc_idx 有效")]
fn aborted_resume_panics_on_out_of_range_index() {
    let mut messages: Vec<ChatNode> = Vec::new();
    let _ = finalize_aborted_parent(&mut messages, 7, "tc-gone");
}

// ── 恢复路的**列表变更入格**（S2.7-b3 前置，ADR-053）──
//
// ## 为什么需要这一批
//
// 主读路径整体切到「从 WAL 重建」之前，必须先回答一个问题：**哪些列表变更还没有
// 对应事实**。恢复路里有三处改写此前只在缓存里发生过：
//
// | 写点 | 只改缓存、不进事实源的后果（切源后） |
// |---|---|
// | `process_retry_subtree` 删 Failed Turn 子树 | 整棵子树**复活**（重建看到的是「它从没被删过」） |
// | `process_tool_resume_action` 删旧子节点 | 同一次调用出现**两条结果**（旧的不消失、新的又来了） |
// | `process_tool_resume_action` 改父 ToolCall | 父节点退回**恢复前**的状态（重建看不到「它被定稿过」） |
//
// 这不是性能问题，是**正确性**问题——与 `t44` 对 CRUD 的判定同一条（编辑回退 /
// 删除复活）。本文件的判据只证「**事实进了事实源**」：缓存此刻仍是读侧真源，
// 故读侧零行为变更；切源那一步的判据属另一个批次（b3 本体）。
//
// 反向自检：把 `process_retry_subtree` 里那段 `v2_messages::removed` 去掉 ⇒ 用例红。

use std::sync::{Arc, Mutex};

use crate::symbio_core::{
    authz::PRINCIPAL_USER, Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_USER_MESSAGE,
};

use super::super::config::SessionConfig;
use super::super::store::SessionStore;
use super::super::types::Session;

/// 帧收集口（`ExecEventSink::direct` 的出口，与 `v2_exec.test.rs` 同款）。
struct CollectingFrames(Mutex<Vec<ChatNode>>);

#[async_trait::async_trait]
impl crate::symbio_core::ExecTranscriptWriter for CollectingFrames {
    async fn apply(&self, m: ChatNode) {
        self.0.lock().unwrap().push(m);
    }
}

/// 装配：持久会话 + 一份「失败轮」缓存（Turn + 一个子节点）+ 一条用户格事实。
///
/// 用户格事实是**必须的**：`v2_messages` 落变更格前要解一个溯源锚，而切片为空时
/// 它宁可不落（见该模块的注释）——种子事实保证切片非空，用例测的才是**落格**这件事。
async fn setup_with_failed_turn() -> (
    Arc<PersistentChatSession>,
    std::path::PathBuf,
    tempfile::TempDir,
) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "resume_test".to_string();

    let mut seed = Session::new(&session_id);
    let failed_turn = ChatNode {
        id: "t-failed".into(),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Skeleton),
        meta: Some(serde_json::json!({"style_tag": "run"})),
        status: Some(NodeStatus::Failed),
        seq: Some(2),
        ..Default::default()
    };
    let partial = ChatNode {
        id: "n-1".into(),
        parent_id: Some("t-failed".into()),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text("半截".into())),
        status: Some(NodeStatus::Completed),
        seq: Some(3),
        ..Default::default()
    };
    seed.messages = vec![
        ChatNode {
            id: "u-1".into(),
            role: Some(NodeRole::User),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text("第一问".into())),
            status: Some(NodeStatus::Completed),
            seq: Some(1),
            ..Default::default()
        },
        failed_turn.clone(),
        partial.clone(),
    ];
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store.session_dir(&session_id).expect("应返回会话目录");
    let session = Arc::new(PersistentChatSession::new(
        session_id,
        Arc::new(tokio::sync::RwLock::new(SessionConfig::default())),
        store,
    ));

    // 种子事实 + **释放写者令牌**（作用域包住）：`v2_messages` 落格时要自己开一次
    // WAL 抢令牌，种子的那份若还活着，它会拿到 `NotTheWriter` ⇒ 事实静默丢弃。
    {
        let wal = dir.join(super::super::paths::V2_WAL_FILE);
        let s = EventWalStore::open(&wal).expect("打开事实源失败");
        s.append(
            Event::pending(
                "u-1",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                1,
                PRINCIPAL_USER,
            )
            .with_payload(json!({ "text": "第一问", "id": "u-1" })),
        )
        .expect("种子事实落格失败");
        // 失败子树也走出生事实：切主后列表 = 事实投影（ADR-049），重试要找的
        // `t-failed` 节点必须**在投影里**才轮得到「删掉一整棵」。直落形状与
        // `v2_messages::added` 产出一致（同一载荷键）。
        let failed = serde_json::to_value(vec![failed_turn.clone(), partial.clone()])
            .expect("序列化种子子树失败");
        s.append(
            Event::pending(
                "m-add-0",
                crate::symbio_core::EVENT_MESSAGE_ADDED,
                Entity::Message,
                Verb::Opened,
                0,
                PRINCIPAL_USER,
            )
            .with_payload(json!({ "messages": failed })),
        )
        .expect("子树事实落格失败");
    }

    (session, dir, tmp)
}

/// 只读读回事实源里的全部事件。
fn events_of(dir: &std::path::Path) -> Vec<Event> {
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    let store = EventWalStore::open_readonly(&wal).expect("只读打开事实源失败");
    store.range(Seq::new(0))
}

/// `message.removed` 的载荷 `ids`（与写方同取法）。
fn ids_of(e: &Event) -> Vec<String> {
    e.payload
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// **重试失败轮**：删掉的是**一整棵子树**，事实源里必须留下「它们没了」这句话。
///
/// 少了它，切到「从 WAL 重建」之后整棵子树复活——而重试的全部意义就是
/// 「重来一遍，不留上次失败的痕迹」。
#[tokio::test]
async fn retry_subtree_lands_a_removed_fact_for_the_whole_subtree() {
    let (session, dir, _tmp) = setup_with_failed_turn().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let req = ResumeRequest {
        target_id: "t-failed".into(),
        action: ResumeAction::RetrySubtree,
        args: None,
        reason: None,
        answer: None,
    };

    super::process_retry_subtree(&session, &sink, &req)
        .await
        .expect("重试失败轮应成功");

    let events = events_of(&dir);
    let removed: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == "message.removed")
        .collect();
    assert_eq!(
        removed.len(),
        1,
        "一次重试落**一条**删除事实（区间用 ids 表达，不是 n 条）：{:?}",
        events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>()
    );
    let ids = ids_of(removed[0]);
    assert_eq!(ids.len(), 2, "Turn 自身 + 它的子节点，一个不漏：{ids:?}");
    assert!(
        ids.contains(&"t-failed".to_string()),
        "Turn 自身在列表里：{ids:?}"
    );
    assert!(ids.contains(&"n-1".to_string()), "子节点在列表里：{ids:?}");
    // 用户格**不在**删除列表里：重试重跑的是同一轮，用户那句话还在历史中
    // （`process_retry_subtree` 的注释与 chat_loop 的续写都依赖这一点）。
    assert!(
        !ids.contains(&"u-1".to_string()),
        "用户发言不该被删：{ids:?}"
    );
}

/// **目标不是 Turn** ⇒ 报 `NotFound`，且**一条事实都不落**。
///
/// 「什么都没删」不该在事实源上留下痕迹——与 `truncate_messages` 空列表不落格、
/// `v2_messages::removed` 自判空列表同一条判据。
#[tokio::test]
async fn retry_subtree_on_a_missing_target_lands_nothing() {
    let (session, dir, _tmp) = setup_with_failed_turn().await;
    let frames: Arc<CollectingFrames> = Arc::new(CollectingFrames(Mutex::new(Vec::new())));
    let sink = crate::symbio_core::ExecEventSink::direct(frames.clone());
    let req = ResumeRequest {
        target_id: "n-1".into(), // 存在，但不是 Turn 节点
        action: ResumeAction::RetrySubtree,
        args: None,
        reason: None,
        answer: None,
    };

    // `ResumeOutcome` 不带 `Debug`（它是**结果**不是诊断载体）⇒ 不用 `expect_err`，
    // 直接解 `Result`。
    let err = match super::process_retry_subtree(&session, &sink, &req).await {
        Ok(_) => panic!("目标不是 Turn ⇒ 应报 NotFound，实际却成功了"),
        Err(e) => e,
    };
    assert!(
        matches!(err, crate::symbio_core::PluginError::NotFound(_)),
        "应是 NotFound（实得 {err:?}）"
    );

    let events = events_of(&dir);
    assert!(
        !events.iter().any(|e| e.kind == "message.removed"),
        "拒绝路径不得落任何变更事实：{:?}",
        events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>()
    );
}
