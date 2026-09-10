//! 文件存储后端测试 —— 顶层平级 + 子会话嵌套存储路由

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

async fn save(store: &FileSessionStore, s: &Session) {
    store.save_session(s).await.unwrap();
}

#[tokio::test]
async fn top_level_sessions_stay_flat() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_aaa", None)).await;

    assert!(tmp.path().join("v2_sess_aaa").join("session.json").exists());
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "v2_sess_aaa");
}

#[tokio::test]
async fn sub_session_routes_into_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path().to_path_buf());
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
    let store = FileSessionStore::new(tmp.path().to_path_buf());
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
