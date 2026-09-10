//! 会话引擎测试 —— `replace_messages` 孤儿存档配对清理。
//!
//! 覆盖三条路径：正常配对删除（L2 压缩语义）、保留新列表仍引用的存档
//! （keep_recent 语义）、以及路径越界防护（`..` 逃逸 / 非 tool_archives 根）。

use super::super::store::create_store;
use super::super::store::StoreKind;
use super::PersistentChatSession;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::ChatSession;
use super::super::types::Session;
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
    store
        .save_session(&seed)
        .await
        .expect("种子会话落盘失败");
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

#[tokio::test]
async fn test_replace_messages_deletes_orphaned_archives() {
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

    // 新列表：只保留对 tool_b 的引用（模拟 L2 压缩 keep_recent 语义）。
    let new_msgs = vec![
        plain_msg("s1"),
        tool_msg_with_archive("m2", kept.to_str().unwrap()),
    ];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(!orphan.exists(), "孤儿存档应被删除: {}", orphan.display());
    assert!(
        kept.exists(),
        "新列表仍引用的存档必须保留: {}",
        kept.display()
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
