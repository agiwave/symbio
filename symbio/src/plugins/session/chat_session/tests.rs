//! 会话引擎测试 —— `replace_messages` 孤儿存档配对清理。
//!
//! 覆盖三条路径：正常配对删除（L2 压缩语义）、保留新列表仍引用的存档
//! （keep_recent 语义）、以及路径越界防护（`..` 逃逸 / 非 tool_archives 根）。

use super::super::store::create_store;
use super::super::store::StoreKind;
use super::super::types::Session;
use super::{prune_historical_tool_calls, ChatSession, PersistentChatSession};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 构造带 `archive_path` meta 的 Tool 消息（模拟 L0 守卫落库后的形态）。
fn tool_msg_with_archive(id: &str, archive_path: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Turn),
        content: Some(MessageContent::Text("已存档".to_string())),
        meta: Some(serde_json::json!({ "archive_path": archive_path })),
        ..Default::default()
    }
}

/// 构造普通文本消息（无存档引用）。
fn plain_msg(id: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("hello".to_string())),
        ..Default::default()
    }
}

/// 建立临时文件存储 + 会话实例；返回 (会话实例, 会话目录, 清理句柄)。
async fn setup() -> (PersistentChatSession, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = create_store(tmp.path().to_path_buf(), StoreKind::File)
        .await
        .expect("文件存储创建失败");
    let session_id = "test_replace_cleanup".to_string();
    // 先落盘一次空会话，使 session_dir 能解析到实际目录。
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store
        .session_dir(&session_id)
        .expect("文件后端应返回会话目录");
    let session = PersistentChatSession::new(
        session_id,
        Arc::new(RwLock::new(SessionConfig::default())),
        store,
    );
    (session, dir, tmp)
}

/// 在会话 tool_archives/ 下创建一个存档文件，返回其绝对路径。
fn make_archive(dir: &Path, name: &str) -> PathBuf {
    let archives = dir.join("tool_archives");
    std::fs::create_dir_all(&archives).expect("tool_archives 创建失败");
    let p = archives.join(name);
    std::fs::write(&p, "archived content").expect("存档写入失败");
    p
}

/// 审计 A3 契约：`replace_messages` 整体重写消息列表时**不删任何存档文件**。
///
/// 归档的磁盘回收唯一归口 `tool_result_guard::prune_archive_dir`（每会话按 mtime
/// 保留最新 `TOOL_ARCHIVE_KEEP` 个）。写入路径历史上会删"失去引用"的孤儿存档，
/// 代价是把不可信的消息 meta 变成删除动作的输入（需要额外的路径逃逸校验），
/// 且与滚动保留职责重叠；本测试锁定"只改消息、不动文件"的新契约。
#[tokio::test]
async fn test_replace_messages_never_deletes_archives() {
    let (session, dir, _tmp) = setup().await;
    let orphan = make_archive(&dir, "tool_a.txt");
    let kept = make_archive(&dir, "tool_b.txt");

    // 旧列表：两条带存档引用的消息。
    let old_msgs = vec![
        tool_msg_with_archive("m1", orphan.to_str().unwrap()),
        tool_msg_with_archive("m2", kept.to_str().unwrap()),
    ];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    // 新列表：丢掉对 tool_a 的引用（模拟 L2 压缩 keep_recent 语义）。
    let new_msgs = vec![
        plain_msg("s1"),
        tool_msg_with_archive("m2", kept.to_str().unwrap()),
    ];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        orphan.exists(),
        "失去引用的存档不得由写入路径删除（交 L0 滚动回收）: {}",
        orphan.display()
    );
    assert!(kept.exists(), "仍被引用的存档必须保留: {}", kept.display());
    let stored = session.get_messages().await.expect("读取存储消息失败");
    assert_eq!(stored.len(), 2, "消息列表应被整体重写");
    assert!(
        !stored.iter().any(|m| m.id == "m1"),
        "被丢弃的消息节点不应留在存储里"
    );
}

#[tokio::test]
async fn test_replace_messages_keeps_archives_when_all_referenced() {
    let (session, dir, _tmp) = setup().await;
    let a = make_archive(&dir, "tool_a.txt");

    let old_msgs = vec![tool_msg_with_archive("m1", a.to_str().unwrap())];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    // 新列表仍引用同一存档（消息 id 变了但引用未丢）。
    let new_msgs = vec![tool_msg_with_archive("m1", a.to_str().unwrap())];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(a.exists(), "仍被引用的存档不应被删除");
}

/// 安全回归：消息 meta 里的 `archive_path` 是**不可信输入**，任何情况下都不得被
/// 写入路径解析成删除动作（含 `..` 相对逃逸）。当前实现根本不删文件，本测试作为
/// 防止将来重新引入"按 meta 删文件"的护栏。
#[tokio::test]
async fn test_replace_messages_rejects_path_traversal() {
    let (session, dir, _tmp) = setup().await;
    // 会话目录外的目标文件（模拟攻击者可写的共享位置）。
    let outside = dir.parent().unwrap().join("outside_secret.txt");
    std::fs::write(&outside, "secret").expect("外部文件写入失败");

    // 旧消息引用 `../outside_secret.txt`（相对路径逃逸）。
    let old_msgs = vec![tool_msg_with_archive("m1", "../outside_secret.txt")];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    let new_msgs = vec![plain_msg("s1")];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        outside.exists(),
        "`..` 逃逸路径必须被拒绝，外部文件不得被删除: {}",
        outside.display()
    );
}

#[tokio::test]
async fn test_replace_messages_rejects_absolute_escape() {
    let (session, dir, _tmp) = setup().await;
    // 绝对路径指向会话目录外的任意文件。
    let outside = dir.parent().unwrap().join("abs_secret.txt");
    std::fs::write(&outside, "secret").expect("外部文件写入失败");

    let old_msgs = vec![tool_msg_with_archive("m1", outside.to_str().unwrap())];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    let new_msgs = vec![plain_msg("s1")];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        outside.exists(),
        "tool_archives/ 根之外的绝对路径必须被拒绝: {}",
        outside.display()
    );
}

// ── 写入期工具链裁剪与配置生效性（审计止血项 P0-2 / P0-3 / P1-②）────────

/// 同 `setup`，但注入自定义会话配置（验证 `prune_tool_history` / `max_messages` 等开关）。
async fn setup_with_config(
    config: SessionConfig,
) -> (PersistentChatSession, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = create_store(tmp.path().to_path_buf(), StoreKind::File)
        .await
        .expect("文件存储创建失败");
    let session_id = "test_config_semantics".to_string();
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store
        .session_dir(&session_id)
        .expect("文件后端应返回会话目录");
    let session = PersistentChatSession::new(session_id, Arc::new(RwLock::new(config)), store);
    (session, dir, tmp)
}

/// 构造一轮完整工具调用链：User → ToolCall → Tool 结果。
fn tool_round(user_id: &str, tc_id: &str, tool_id: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            id: user_id.to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(format!("q-{}", user_id))),
            ..Default::default()
        },
        ChatMessage {
            id: tc_id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            parent_id: Some(user_id.to_string()),
            meta: Some(serde_json::json!({ "tool_name": "local/sh" })),
            ..Default::default()
        },
        ChatMessage {
            id: tool_id.to_string(),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Turn),
            parent_id: Some(tc_id.to_string()),
            content: Some(MessageContent::Text(format!("result-{}", tool_id))),
            ..Default::default()
        },
    ]
}

/// 回归（P0-2）：`keep_turns = 0`（语义"不限制"）不得越界 panic。
/// 旧实现直接取 `user_indices[len - 0]` → 下标 `len` 越界。
#[tokio::test]
async fn prune_zero_keep_turns_does_not_panic() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    prune_historical_tool_calls(&mut msgs, 0);

    assert_eq!(msgs.len(), 6, "keep_turns=0 应保留全部消息（不限制）");
    assert!(msgs.iter().any(|m| m.id == "r1"));
    assert!(msgs.iter().any(|m| m.id == "r2"));
}

/// 回归（P0-2）：空列表 + `keep_turns = 0` 同样是空操作。
#[tokio::test]
async fn prune_empty_messages_zero_keep_turns_is_noop() {
    let mut msgs: Vec<ChatMessage> = Vec::new();
    prune_historical_tool_calls(&mut msgs, 0);
    assert!(msgs.is_empty());
}

/// 正常语义：窗口外的工具链被物理删除，窗口内保留。
#[tokio::test]
async fn prune_keeps_recent_turns_and_drops_older_tool_chain() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    prune_historical_tool_calls(&mut msgs, 1);

    assert!(
        !msgs.iter().any(|m| m.id == "r1"),
        "最近 1 轮之外的工具结果应被裁剪"
    );
    assert!(
        !msgs.iter().any(|m| m.id == "t1"),
        "对应的 ToolCall 应一并裁剪"
    );
    assert!(msgs.iter().any(|m| m.id == "r2"), "最近一轮工具结果应保留");
}

/// 回归（审计 A3）：写入期裁剪**只删节点、不删文件**。
///
/// 旧实现在淘汰旧轮消息时按 `meta.archive_path` 物理删除 L0 存档，既把不可信的
/// 消息 meta 当删除依据，又与 `tool_result_guard` 的每会话滚动保留职责冲突（被淘汰
/// 的"旧轮"消息往往持有最新存档，反而被误删）。文件回收统一归口 `replace_messages`
/// 的孤儿清理 + 守卫自身的滚动保留。
#[tokio::test]
async fn prune_drops_nodes_but_never_deletes_archive_files() {
    let (session, dir, _tmp) = setup_with_config(SessionConfig {
        prune_tool_history: true,
        context_messages: 1,
        ..Default::default()
    })
    .await;
    let archive = make_archive(&dir, "prune_me.txt");

    // 两轮工具链，第一轮持有真实存档文件。
    let mut msgs = tool_round("u1", "t1", "r1");
    msgs[2].meta = Some(serde_json::json!({ "archive_path": archive.to_str().unwrap() }));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let stored = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !stored.iter().any(|m| m.id == "r1"),
        "窗口外的工具结果节点应被裁剪"
    );
    assert!(
        archive.exists(),
        "被裁剪节点的存档文件不得由写入路径删除: {}",
        archive.display()
    );
}

/// 回归（P0-2）：`context_messages = 0` 走完整 `append_messages` 链路也不得失败。
#[tokio::test]
async fn append_messages_with_zero_context_messages_does_not_fail() {
    let config = SessionConfig {
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    session
        .append_messages(msgs)
        .await
        .expect("context_messages=0 时落库不得失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        ctx.iter().any(|m| m.id == "r1"),
        "0 = 不限制：窗口外的工具结果仍应留在存储中"
    );
}

/// 新开关（P1-②）：`prune_tool_history = false` 时存储保留完整工具链。
#[tokio::test]
async fn append_messages_keeps_tool_chain_when_prune_disabled() {
    let config = SessionConfig {
        context_messages: 1,
        prune_tool_history: false,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        ctx.iter().any(|m| m.id == "r1"),
        "关闭写入期裁剪后，窗口外的工具结果应保留在存储中"
    );
}

/// 默认行为不回退：`prune_tool_history = true` 时窗口外工具链仍被物理裁剪。
#[tokio::test]
async fn append_messages_prunes_tool_chain_by_default() {
    let config = SessionConfig {
        context_messages: 1,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !ctx.iter().any(|m| m.id == "r1"),
        "默认开启写入期裁剪：窗口外的工具结果应被删除"
    );
}

/// 回归（P0-3）：`max_messages` 不再被存储层 `.max(500)` 硬下限钳制，小值真实生效。
#[tokio::test]
async fn max_messages_below_legacy_store_floor_is_honored() {
    let config = SessionConfig {
        max_messages: 2,
        prune_tool_history: false,
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(plain_msg(&format!("u{}", i)));
    }
    session.append_messages(msgs).await.expect("落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !ctx.iter().any(|m| m.id == "u1"),
        "max_messages=2 应触发 FIFO 淘汰；被 500 下限钳制时 u1 会留在存储中"
    );
    assert!(
        ctx.len() < 5,
        "max_messages=2 应裁剪存储轮次，实际保留 {} 条",
        ctx.len()
    );
    assert!(ctx.iter().any(|m| m.id == "u5"), "最近一轮用户消息必须保留");
}

/// `max_messages = 0`：不限制，全部保留。
#[tokio::test]
async fn max_messages_zero_means_unlimited() {
    let config = SessionConfig {
        max_messages: 0,
        prune_tool_history: false,
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(plain_msg(&format!("u{}", i)));
    }
    session.append_messages(msgs).await.expect("落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert_eq!(ctx.len(), 5, "max_messages=0 表示不限制");
}
