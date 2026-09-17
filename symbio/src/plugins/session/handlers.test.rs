//! `handlers.rs` 删除路由的测试 —— 级联删除的**变更语义**。
//!
//! 删除一条消息会连带删掉它之后的全部消息（`messages.drain(i..)`）。这段区间必须以
//! **一条** `truncated` 变更下发，而不是 N 条 `deleted`：
//!
//! 1. **可分辨**——`deleted` 的语义是「**这一个**节点没了」（工具调用恢复时删旧子节点
//!    走的正是它，删完后面还有父节点的最终回答）。两者若共用同一个值，消费者收到的
//!    每一条都长得一样，「删这一个」与「从这里删到末尾」只能靠外部知识去猜；
//! 2. **代价**——逐条下发的变更数与历史长度线性相关：删一条早期消息要发上百条。
//!
//! 本文件锁定这两点，以及一条边界：目标消息不存在时**一条变更都不发**
//! （「什么都没删」不该在 VDFS 上留下痕迹——发了 `truncated` 会让消费者从一条并不
//! 存在的节点起截断，把整个列表清空）。

use super::super::plugin::message_path;
use super::super::plugin::SessionPlugin;
use super::super::types::Session;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::vdfs::VdfsChange;
use crate::symbio_core::{InvokeRequest, SimpleRequest};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// 每例独占存储根；guard 在插件之后释放，失败时也会清理。
fn fixture() -> (tempfile::TempDir, SessionPlugin) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = SessionPlugin::new(
        None,
        super::super::config::SessionConfig::default(),
        crate::symbio_core::PluginDir::at(dir.path(), "session"),
    );
    (dir, plugin)
}

fn msg(id: &str, role: MessageRole, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(role),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 落一条空会话 + 四条消息（`seq` 由 `replace_messages` 按数组顺序分配）。
async fn seed(p: &SessionPlugin, sid: &str) {
    let store = p.get_store().await.expect("存储不可用");
    store
        .save_session(&Session::new(sid))
        .await
        .expect("种子会话落盘失败");
    let session = p.open_chat_session(sid).await.expect("会话打开失败");
    session
        .replace_messages(vec![
            msg("m1", MessageRole::User, "问题一"),
            msg("m2", MessageRole::User, "问题二"),
            msg("m3", MessageRole::Assistant, "回答二"),
            msg("m4", MessageRole::User, "问题三"),
        ])
        .await
        .expect("消息落盘失败");
}

/// 收集订阅表投递过来的全部变更（`VdfsChangeSink` 是 `Fn`，故用 `Mutex` 兜内部可变）。
fn spy(p: &SessionPlugin) -> Arc<Mutex<Vec<VdfsChange>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    p.change_subs
        .watch("", Arc::new(move |c| sink.lock().unwrap().push(c)));
    seen
}

/// 构造带 payload 的请求上下文（`InvokeRequest::payload` 读的正是 `"payload"` 桶）。
fn ctx_with(payload: serde_json::Value) -> Arc<dyn InvokeRequest> {
    let req = SimpleRequest::new(None, None);
    req.extensions
        .write()
        .unwrap()
        .insert("payload".to_string(), Arc::new(payload));
    Arc::new(req)
}

fn deleted_ids_of(res: &serde_json::Value) -> Vec<String> {
    res["deleted_ids"]
        .as_array()
        .expect("deleted_ids 必须是数组")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn cascade_delete_emits_one_truncated_change_not_n_deleted() {
    let (_dir, p) = fixture();
    let sid = "cascade";
    seed(&p, sid).await;
    let seen = spy(&p);

    let res = p
        .invoke_delete_message(ctx_with(json!({ "session_id": sid, "message_id": "m2" })))
        .await
        .expect("删除失败");

    // 后端语义：目标及其之后的全部消息
    assert_eq!(deleted_ids_of(&res), vec!["m2", "m3", "m4"]);

    let got = seen.lock().unwrap();
    assert_eq!(
        got.len(),
        1,
        "级联删除必须只发一条变更——逐条 `deleted` 的代价与历史长度线性相关，\
         且消费者无从分辨它和「只删这一个」"
    );
    assert_eq!(got[0].change, "truncated");
    assert_eq!(
        got[0].path,
        message_path(sid, "m2"),
        "变更落在**目标消息**的地址上：区间起点由它给出，其余由消费者按自己的顺序取"
    );
}

#[tokio::test]
async fn deleting_the_last_message_still_truncates_from_its_own_address() {
    let (_dir, p) = fixture();
    let sid = "tail";
    seed(&p, sid).await;
    let seen = spy(&p);

    let res = p
        .invoke_delete_message(ctx_with(json!({ "session_id": sid, "message_id": "m4" })))
        .await
        .expect("删除失败");

    assert_eq!(deleted_ids_of(&res), vec!["m4"]);
    let got = seen.lock().unwrap();
    // 只删末尾一条也走同一条语义：消费者不必区分「截断到末尾」与「截掉一个尾巴」，
    // 两种情况下"取该节点及其后"都是对的。
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].change, "truncated");
    assert_eq!(got[0].path, message_path(sid, "m4"));
}

#[tokio::test]
async fn deleting_an_unknown_message_emits_nothing() {
    let (_dir, p) = fixture();
    let sid = "missing";
    seed(&p, sid).await;
    let seen = spy(&p);

    let res = p
        .invoke_delete_message(ctx_with(
            json!({ "session_id": sid, "message_id": "不存在" }),
        ))
        .await
        .expect("删除失败");

    assert!(deleted_ids_of(&res).is_empty());
    assert!(
        seen.lock().unwrap().is_empty(),
        "什么都没删就不该发变更：`truncated` 会让消费者从一条并不存在的节点起截断，\
         把整个列表清空"
    );
}
