//! `handlers.rs` 遗留面的测试。
//!
//! ## 原先在这里的三个用例去哪了
//!
//! 本文件曾有「级联删除的变更语义」三例，测的是 `invoke_delete_message`。那条路由
//! （连同实现与 schema）已于 2026-09-18 退役，消息删除改走
//! `action(<id>/message/<mid>, "truncate")`——**契约随实现一起搬到了**
//! `plugin/vdfs_provider.test.rs`：
//!
//! | 原用例锁定的契约 | 现位置 |
//! |---|---|
//! | 级联删除发**一条** `truncated`，不是 N 条 `deleted` | `truncate_notifies_once_on_the_starting_message` |
//! | 回执给出**权威**的被删 id 列表 | `truncate_removes_the_target_and_everything_after` |
//! | 目标不存在 ⇒ **一条变更都不发** | `truncate_of_missing_target_changes_nothing` |
//!
//! 是**搬移**不是重写：契约没变，只是入口从专用路由换成了节点动作。
//!
//! ## 会话 metadata 的写入：从「两条路径一致性」到「一条路径」
//!
//! 这里曾有 `session_update_and_vdfs_write_agree_on_metadata`，锁的是
//! `session/update` 路由与 `vdfs/write` 产出**逐字相同**的 metadata。那条路由已于
//! 2026-09-23 退役（CLI 改走 `vdfs/write`），于是「一致性」不再是一个可断言的性质
//! ——只剩一条路径，分歧在结构上写不出来。契约本身没有消失，搬到了
//! `plugin/vdfs_provider.test.rs`：
//!
//! | 原用例锁定的契约 | 现位置 |
//! |---|---|
//! | `metadata` 是**浅合并**（未提到的键保持不变） | `write_merges_metadata_shallowly` |
//! | 显式 `title` 写进 `metadata.title` | 同上 |
//!
//! ## 留在这里的一件事
//!
//! 已退役的路由**不得被加回来**。

use super::super::plugin::SessionPlugin;
use crate::symbio_core::{InvokeRequest, SimpleRequest};
use serde_json::json;
use std::sync::Arc;

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

/// 构造带 payload 的请求上下文（`InvokeRequest::payload` 读的正是 `"payload"` 桶）。
fn ctx_with(payload: serde_json::Value) -> Arc<dyn InvokeRequest> {
    let req = SimpleRequest::new(None, None);
    req.extensions
        .write()
        .unwrap()
        .insert("payload".to_string(), Arc::new(payload));
    Arc::new(req)
}

// ==================== 退役路由：不得被加回来 ====================

/// `session/clear` 路由已退役：删除会话的唯一入口是
/// `vdfs/delete(<根>/session/<id>)`。
///
/// 为什么值得锁：退役一条路由**不会**让任何既有测试变红——调用方全改完了，剩下的
/// 只是一个不再被解析的字符串。若哪天有人"顺手"把它加回来，同一件事就又有了两个
/// 入口、两条会各自漂移的实现，而没有任何测试会覆盖它们的一致性。
///
/// 断言**错误消息**而不只是错误类型：变异测试时发现，把 `"clear"` 加回去却让它
/// 返回 `NotFound` 也能骗过"只查类型"的断言——而那种写法与"没有这条路由"行为完全
/// 相同，根本不是回归。真正要锁的是「`clear` 落到了**默认分支**」，那正是
/// `未知路径` 这条消息的出处。
#[tokio::test]
async fn session_clear_route_is_retired() {
    use crate::symbio_core::{InvokeRequestExt, Plugin};

    let (_dir, p) = fixture();
    let ctx = ctx_with(json!({ "session_id": "s1" }));
    ctx.set(crate::symbio_core::PATH, "clear".to_string());

    let err = Plugin::route(std::sync::Arc::new(p), ctx)
        .await
        .expect_err("session/clear 已退役，不该再被解析");

    match err {
        crate::symbio_core::PluginError::NotFound(msg) => assert!(
            msg.contains("未知路径"),
            "clear 应落到默认分支（未知路径），实际消息：{msg}"
        ),
        other => panic!("应报 NotFound（未知路径），实际：{other:?}"),
    }
}

/// 已退役的会话路由**不得被加回来**：2026-09-18 迁往 VDFS 的五条 + 2026-09-23 的
/// `get_messages`（存在性校验改走进程内 VDFS 纯接口 `get_vfs_provider` + `stat`）
/// 与 `update`（会话 metadata 写入收敛为 `vdfs/write`）。
///
/// 每条路径现在都有一个 VDFS 入口（映射见 `docs/legacy-route-migration.md`）。
/// 与 `session/clear` 同理：退役不会让任何既有测试变红，因此需要一条**正向**的
/// 断言把「这些字符串不再被解析」钉住，否则它们会悄悄长回来。
#[tokio::test]
async fn migrated_session_routes_stay_retired() {
    use crate::symbio_core::{InvokeRequestExt, Plugin};

    let (_dir, p) = fixture();
    let p = Arc::new(p);
    for (path, successor) in [
        // 内部调用已改为直连引擎；「发言」始终只走聊天协议
        ("append", "open_chat_session + append_messages"),
        // 无消费方，整条链路（路由 + 实现 + schema）已删
        ("open", "（无替代：本就不需要）"),
        ("chat/update_message", "vdfs/write(<sid>/message/<mid>)"),
        (
            "chat/delete_message",
            "vdfs/action(<sid>/message/<mid>, \"truncate\")",
        ),
        (
            "chat/clear_messages",
            "vdfs/action(<sid>/message, \"clear\")",
        ),
        (
            "get_messages",
            "进程内 vdfs/stat（get_vfs_provider + stat(<挂载名>/<sid>)）",
        ),
        // 客户端指定会话 id 由「具名目标 + create」承担，不再需要专用路由
        (
            "update",
            "vdfs/write(<根>/session/<id>, {create:true, metadata})",
        ),
    ] {
        let ctx = ctx_with(json!({ "session_id": "s1" }));
        ctx.set(crate::symbio_core::PATH, path.to_string());

        let err = Plugin::route(Arc::clone(&p), ctx)
            .await
            .expect_err("已退役的路由不该再被解析");

        match err {
            crate::symbio_core::PluginError::NotFound(msg) => assert!(
                msg.contains("未知路径"),
                "{path} 应落到默认分支（未知路径），实际消息：{msg}\
                 （它的后继是 {successor}）"
            ),
            other => panic!("{path} 应报 NotFound（未知路径），实际：{other:?}"),
        }
    }
}
