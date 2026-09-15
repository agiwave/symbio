//! 会话存储测试 —— 顶层平级 + 子会话嵌套路由 + 两种驻留方式的契约一致
//!
//! 第三类断言（`ephemeral_*`）的意义：证明「临时会话复用同一份会话引擎实现」是
//! 安全的——不落盘型在 load 缺省语义、save upsert、delete、list 排序上与落盘型
//! 逐条一致，上层 `PersistentChatSession` 无需为它特判（审计 B2 的原始理由）。

use super::*;
use serde_json::json;
use tempfile::TempDir;

fn session_with(id: &str, parent: Option<&str>) -> Session {
    let mut s = Session::new(id);
    if let Some(p) = parent {
        s.metadata["parent_session_id"] = json!(p);
    }
    s.updated_at = 1000;
    s
}

async fn save(store: &SessionStore, s: &Session) {
    store.save_session(s).await.unwrap();
}

#[tokio::test]
async fn top_level_sessions_stay_flat() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_aaa", None)).await;

    assert!(tmp.path().join("v2_sess_aaa").join("session.json").exists());
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "v2_sess_aaa");
}

#[tokio::test]
async fn sub_session_routes_into_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_parent", None)).await;
    save(
        &store,
        &session_with("v2_sess_child", Some("v2_sess_parent")),
    )
    .await;

    // 嵌套目录布局：<parent>/sessions/<child>/session.json
    assert!(tmp
        .path()
        .join("v2_sess_parent")
        .join("sessions")
        .join("v2_sess_child")
        .join("session.json")
        .exists());

    // 子会话不出现在顶层清单
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);

    // 子会话清单：仅父目录内的嵌套会话
    let subs = store.list_sub_sessions("v2_sess_parent").await.unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, "v2_sess_child");
    assert!(store
        .list_sub_sessions("v2_sess_noparent")
        .await
        .unwrap()
        .is_empty());

    // 子会话可凭自身 id 寻址（load / session_dir / 删除）
    let loaded = store.load_session("v2_sess_child").await.unwrap();
    assert_eq!(loaded.id, "v2_sess_child");
    assert_eq!(
        store.session_dir("v2_sess_child").unwrap(),
        tmp.path()
            .join("v2_sess_parent")
            .join("sessions")
            .join("v2_sess_child")
    );
    store.delete_session("v2_sess_child").await.unwrap();
    assert!(store
        .list_sub_sessions("v2_sess_parent")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn deleting_parent_cascades_sub_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_parent", None)).await;
    save(
        &store,
        &session_with("v2_sess_child1", Some("v2_sess_parent")),
    )
    .await;
    save(
        &store,
        &session_with("v2_sess_child2", Some("v2_sess_parent")),
    )
    .await;

    store.delete_session("v2_sess_parent").await.unwrap();
    assert!(store.list_sessions().await.unwrap().is_empty());
    assert!(store
        .list_sub_sessions("v2_sess_parent")
        .await
        .unwrap()
        .is_empty());
}

/// 不落盘型与落盘型**逐条同构**：会话引擎因此只有一份实现
#[tokio::test]
async fn ephemeral_store_matches_the_persistent_contract() {
    let store = SessionStore::ephemeral();

    // load 不存在的会话 → 新建空 Session（不报错、不留下任何东西）
    let fresh = store.load_session("mem_missing").await.unwrap();
    assert_eq!(fresh.id, "mem_missing");
    assert!(store.list_sessions().await.unwrap().is_empty());

    // save → load 往返；load 返回克隆，调用方在副本上改写不影响库内那份
    let mut s = session_with("mem_a", None);
    s.metadata["k"] = json!("v");
    save(&store, &s).await;
    let loaded = store.load_session("mem_a").await.unwrap();
    assert_eq!(loaded.metadata["k"], "v");

    // 同 id 再存 → upsert 而非重复
    s.updated_at = 2000;
    save(&store, &s).await;
    assert_eq!(
        store.list_sessions().await.unwrap().len(),
        1,
        "mem_a 的两次写不应变成两条"
    );

    // list 按 updated_at 降序（与落盘型同一份排序规则）
    save(&store, &session_with("mem_b", None)).await; // 1000
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "mem_a", "2000 在前");
    assert_eq!(listed[1].id, "mem_b");

    // delete 后回到"空 Session"语义，清单里只剩没被删的那条
    store.delete_session("mem_a").await.unwrap();
    assert!(store
        .load_session("mem_a")
        .await
        .unwrap()
        .messages
        .is_empty());
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "mem_b");

    // 无目录概念：存档路径解析退化为"无存档"，子会话清单恒空
    assert!(store.session_dir("mem_a").is_none());
    assert!(store.list_sub_sessions("mem_a").await.unwrap().is_empty());
}

/// 每个临时会话各一张表：句柄之间不串台
#[tokio::test]
async fn each_ephemeral_store_gets_its_own_table() {
    let a = SessionStore::ephemeral();
    let b = SessionStore::ephemeral();
    save(&a, &session_with("shared", None)).await;

    assert_eq!(a.list_sessions().await.unwrap().len(), 1);
    assert!(
        b.list_sessions().await.unwrap().is_empty(),
        "临时会话的可见范围就是它自己的句柄生命周期"
    );
}

/// 截断自愈：并发写交错留下的「短 JSON + 旧内容残留」必须还能读出首个完整会话
///
/// 这是**用户历史**的容错路径，此前无测试钉住。
#[tokio::test]
async fn truncated_session_json_self_heals() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_heal", None)).await;

    let path = file_for(tmp.path(), "v2_sess_heal");
    let good = std::fs::read_to_string(&path).unwrap();
    // 造一份「完整 JSON 之后又粘了半个对象」的坏文件（trailing characters 形态）
    std::fs::write(&path, format!("{good}{{\"id\":\"residue\"")).unwrap();

    let healed = store.load_session("v2_sess_heal").await.unwrap();
    assert_eq!(healed.id, "v2_sess_heal", "首个完整对象应被读出");

    // 彻底坏掉（首个对象都不完整）时才是解析失败，不能静默当成空会话
    std::fs::write(&path, "{\"id\":\"v2_sess_heal\",\"messages\":[{").unwrap();
    assert!(matches!(
        store.load_session("v2_sess_heal").await.unwrap_err(),
        PluginError::ParseError(_)
    ));
}

/// 原子写的收尾：成功路径上不得留下 `.tmp`（否则残留会随每次保存累积）
#[tokio::test]
async fn save_leaves_no_temp_file_behind() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_atomic", None)).await;
    save(&store, &session_with("v2_sess_atomic", None)).await;

    let dir = dir_for(tmp.path(), "v2_sess_atomic");
    assert!(dir.join(SESSION_FILE).is_file());
    assert!(
        !dir.join(SESSION_FILE_TMP).exists(),
        "rename 后临时文件不该存在"
    );
}

/// id 里的分隔符 / `..` 不得让会话目录逃出存储根
///
/// 这条防护是把宿主层 `safe_segment` 接进会话侧（paths::safe_id）之后才有的——
/// 此前会话侧只替换 `/ \ :`，`.` 与 `..` 可以原样成为目录名。
#[tokio::test]
async fn hostile_ids_cannot_escape_the_storage_root() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    for id in ["../../escape", "..", ".", "a/../../b"] {
        save(&store, &session_with(id, None)).await;
        let dir = store.session_dir(id).unwrap().canonicalize().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        assert!(
            dir.starts_with(&root),
            "id `{id}` 落到了根外：{}",
            dir.display()
        );
        assert_eq!(
            store.load_session(id).await.unwrap().id,
            id,
            "归一化只影响段名，不影响 id 本身"
        );
    }

    // 根下只多出归一化后的目录，没有任何名为 `..` / `.` 的条目
    assert!(!tmp.path().join("..").join("escape").exists());
}
