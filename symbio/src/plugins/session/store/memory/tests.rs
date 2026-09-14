//! 内存存储后端测试 —— 与文件 / SQLite 后端共享的 `SessionStore` 契约
//!
//! 这些断言的意义：证明"临时会话复用同一份会话引擎实现"是安全的——内存后端
//! 在 load 缺省语义、save upsert、delete、list 排序上与持久化后端逐条一致，
//! 上层 `PersistentChatSession` 无需为本后端特判（审计 B2）。

use super::*;
use crate::plugins::session::store::SessionStore;

fn session_with(id: &str) -> Session {
    let mut s = Session::new(id);
    s.updated_at = 1000;
    s
}

#[tokio::test]
async fn memory_store_roundtrip_contract() {
    let store = InMemorySessionStore::new();

    // load 不存在的会话 → 新建空 Session（与文件后端一致，不报错、不落库）
    let fresh = store.load_session("mem_missing").await.unwrap();
    assert_eq!(fresh.id, "mem_missing");
    assert!(store.list_sessions().await.unwrap().is_empty());

    // save → load 往返：load 返回克隆，调用方改写不影响库内副本
    let mut s = session_with("mem_a");
    s.metadata["k"] = serde_json::json!("v");
    store.save_session(&s).await.unwrap();
    let loaded = store.load_session("mem_a").await.unwrap();
    assert_eq!(loaded.metadata["k"], "v");

    // 同 id 再存 → upsert 而非报错/重复
    s.updated_at = 2000;
    store.save_session(&s).await.unwrap();
    assert_eq!(store.list_sessions().await.unwrap().len(), 1);

    // list 按 updated_at 降序（与文件 / SQLite 后端一致）
    store.save_session(&session_with("mem_b")).await.unwrap(); // 1000
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed[0].id, "mem_a"); // 2000 在前
    assert_eq!(listed[1].id, "mem_b");

    // delete 后 load 回到"空 Session"语义
    store.delete_session("mem_a").await.unwrap();
    assert!(store
        .load_session("mem_a")
        .await
        .unwrap()
        .messages
        .is_empty());
    assert_eq!(store.list_sessions().await.unwrap().len(), 1);
}

/// 无目录概念：`session_dir` 返回 None（与 SQLite 后端一致），
/// 存档路径解析因此退化为"无存档"。
#[tokio::test]
async fn memory_store_has_no_session_dir() {
    let store = InMemorySessionStore::new();
    store.save_session(&session_with("mem_dir")).await.unwrap();
    assert!(store.session_dir("mem_dir").is_none());
}
