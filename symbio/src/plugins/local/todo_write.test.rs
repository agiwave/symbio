//! `todo_write` 的合并语义与作用域边界。
//!
//! 工具内存是**进程作用域**（D-133）：这里钉的是「同一 key 内怎么合并」与
//! 「不同 key 互不相干」；跨进程的持久真相是 `task.*` 事实，由 `task_events`
//! 从事实快照重建，不在本文件的范围。

use super::*;
use serde_json::json;

fn args(todos: serde_json::Value, merge: bool) -> Value {
    json!({ "todos": todos, "merge": merge })
}

#[tokio::test]
async fn replace_drops_items_not_in_the_new_list() {
    let tool = TodoWriteTool;
    let key = "test/todo-write/replace";
    tool.execute_inner(
        &args(json!([{ "id": "a", "content": "旧任务" }]), false),
        key,
    )
    .await
    .unwrap();
    let out = tool
        .execute_inner(
            &args(json!([{ "id": "b", "content": "新任务" }]), false),
            key,
        )
        .await
        .unwrap();
    assert_eq!(out["count"], 1, "整体替换后只剩新清单的那一项");
}

#[tokio::test]
async fn merge_updates_by_id_and_appends_unseen_ids() {
    let tool = TodoWriteTool;
    let key = "test/todo-write/merge";
    tool.execute_inner(
        &args(
            json!([
                { "id": "a", "content": "第一步", "status": "pending" },
                { "id": "b", "content": "第二步", "status": "pending" }
            ]),
            false,
        ),
        key,
    )
    .await
    .unwrap();
    let out = tool
        .execute_inner(
            &args(
                json!([
                    { "id": "a", "content": "第一步", "status": "completed" },
                    { "id": "c", "content": "第三步", "status": "pending" }
                ]),
                true,
            ),
            key,
        )
        .await
        .unwrap();
    assert_eq!(out["count"], 3, "同 id 覆盖、未见过 id 追加");
}

#[tokio::test]
async fn a_fresh_key_starts_empty() {
    // 进程作用域的边界：换一个 key（= 另一个会话 / 进程重启后的同会话）拿不到
    // 上一份内存清单。持久状态在 `task.*` 事实里，不在工具里（D-133）。
    let tool = TodoWriteTool;
    tool.execute_inner(
        &args(json!([{ "id": "a", "content": "旧会话的任务" }]), false),
        "test/todo-write/session-a",
    )
    .await
    .unwrap();
    let out = tool
        .execute_inner(
            &args(json!([{ "id": "b", "content": "新会话的任务" }]), true),
            "test/todo-write/session-b",
        )
        .await
        .unwrap();
    assert_eq!(out["count"], 1, "不同 key 不共享内存清单");
}

#[tokio::test]
async fn missing_content_is_rejected() {
    let tool = TodoWriteTool;
    let err = tool
        .execute_inner(
            &args(json!([{ "id": "a" }]), false),
            "test/todo-write/invalid",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, PluginError::ValidationError(_)),
        "缺 content 是调用方错误，不是空任务"
    );
}
