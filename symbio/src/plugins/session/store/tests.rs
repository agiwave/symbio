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

/// `load_session_checked` 是存在性判据：命中给 `Some`、未命中给 `None`（而不是空
/// 会话），且**按 id 直取**——嵌套子会话同样命中，无需先列全量清单再 `find`
#[tokio::test]
async fn load_session_checked_distinguishes_missing_from_empty() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("v2_sess_parent", None)).await;
    save(
        &store,
        &session_with("v2_sess_child", Some("v2_sess_parent")),
    )
    .await;

    // 命中：顶层与嵌套子会话都凭自身 id 直取
    for id in ["v2_sess_parent", "v2_sess_child"] {
        let got = store.load_session_checked(id).await.unwrap();
        assert_eq!(got.map(|s| s.id), Some(id.to_string()), "{id} 应命中");
    }

    // 未命中：`None`——这正是 `load_session`（返回空会话）无法给出的区分
    assert!(store
        .load_session_checked("v2_sess_none")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.load_session("v2_sess_none").await.unwrap().id,
        "v2_sess_none",
        "同一份存储上 load_session 仍按「未命中 = 空会话」回答"
    );

    // 不落盘型同契约
    let mem = SessionStore::ephemeral();
    assert!(mem
        .load_session_checked("v2_sess_none")
        .await
        .unwrap()
        .is_none());
    save(&mem, &session_with("v2_sess_mem", None)).await;
    assert!(mem
        .load_session_checked("v2_sess_mem")
        .await
        .unwrap()
        .is_some());
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

// ==================== 元数据 / 消息分文件（清单性能） ====================
//
// 这一组断言钉住的是「列一次清单的成本与会话聊了多久无关」：清单只读
// `session.json`，`messages.json` 只有真的要取转写时才读。

/// 造一条用户文本消息（标题 / 摘要推导的输入）
fn text_msg(role_is_user: bool, text: &str) -> ChatMessage {
    use crate::symbio_core::schemas::session::chat_message::{MessageContent, MessageRole};
    ChatMessage {
        id: format!("m{}", text.len()),
        role: Some(if role_is_user {
            MessageRole::User
        } else {
            MessageRole::Assistant
        }),
        content: Some(MessageContent::Text(text.into())),
        ..Default::default()
    }
}

fn session_with_messages(id: &str, updated_at: i64) -> Session {
    let mut s = Session::new(id);
    s.messages = vec![
        text_msg(true, "帮我看看这个仓库的结构"),
        text_msg(false, "好的，我先看一下目录。"),
    ];
    s.updated_at = updated_at;
    s
}

/// 保存拆成两个文件：元数据里**不再**内联消息
#[tokio::test]
async fn save_splits_metadata_and_messages() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with_messages("v2_sess_split", 1000)).await;

    let dir = dir_for(tmp.path(), "v2_sess_split");
    assert!(dir.join(SESSION_FILE).is_file());
    assert!(dir.join(MESSAGES_FILE).is_file());

    let meta_text = std::fs::read_to_string(dir.join(SESSION_FILE)).unwrap();
    assert!(
        !meta_text.contains("\"messages\""),
        "元数据里不该再内联消息：{meta_text}"
    );
    let msgs_text = std::fs::read_to_string(dir.join(MESSAGES_FILE)).unwrap();
    assert!(msgs_text.contains("帮我看看这个仓库的结构"));

    // 往返：消息一条不少
    let loaded = store.load_session("v2_sess_split").await.unwrap();
    assert_eq!(loaded.messages.len(), 2);
}

/// 清单只读元数据：**消息文件坏了也不影响列清单**（这是拆分的核心收益）
#[tokio::test]
async fn list_sessions_does_not_touch_messages() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with_messages("v2_sess_ok", 1000)).await;

    // 把消息文件写成垃圾：清单仍然列得出来
    std::fs::write(
        dir_for(tmp.path(), "v2_sess_ok").join(MESSAGES_FILE),
        "{ not json",
    )
    .unwrap();

    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "v2_sess_ok");
    // 投影还在（保存时算好的），因此清单不受影响
    assert_eq!(listed[0].message_count, 2);
}

/// 保存时算好投影：标题 / 条数 / 摘要 / 标签都不必读消息就能拿到
#[tokio::test]
async fn save_persists_the_list_projection() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    let mut s = session_with_messages("v2_sess_proj", 1000);
    s.metadata["workdir"] = json!("/tmp/proj/demo");
    save(&store, &s).await;

    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    let p = &listed[0];
    // 标题 = 第一条用户消息的首行（限长）
    assert_eq!(p.title, "帮我看看这个仓库的结构");
    assert_eq!(p.message_count, 2);
    assert_eq!(p.summary.as_deref(), Some("好的，我先看一下目录。"));
    assert_eq!(p.meta_tags, vec!["demo".to_string(), "2 条".to_string()]);
}

/// **存量文件**（消息内联、没有投影字段）照样可读，且清单给的是友好名，
/// 不是 id —— 这条专门钉住「列表退化成一串短 guid」那次回归。
#[tokio::test]
async fn legacy_inline_session_keeps_a_friendly_title() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    // 手写一个旧布局文件：消息内联，**没有** title / message_count / summary / meta_tags
    let dir = dir_for(tmp.path(), "v2_sess_legacy");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SESSION_FILE),
        r#"{"id":"v2_sess_legacy","messages":[{"id":"m1","role":"user","content":"老会话的标题"},{"id":"m2","role":"assistant","content":"收到"}],"created_at":1,"updated_at":2,"metadata":{}}"#,
    )
    .unwrap();

    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].title, "老会话的标题",
        "投影缺失时必须用内联消息就地补算，否则清单会显示 id"
    );
    assert_ne!(listed[0].title, listed[0].id);
    assert_eq!(listed[0].message_count, 2);

    // 消息也读得回来（旧布局没有 messages.json）
    let loaded = store.load_session("v2_sess_legacy").await.unwrap();
    assert_eq!(loaded.messages.len(), 2);
}

/// 迁移：旧布局 → 两文件，并补写投影；**幂等**（第二次跑不再改动）
#[tokio::test]
async fn migrate_splits_legacy_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    let dir = dir_for(tmp.path(), "v2_sess_mig");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SESSION_FILE),
        r#"{"id":"v2_sess_mig","messages":[{"id":"m1","role":"user","content":"迁移前的话"}],"created_at":1,"updated_at":2,"metadata":{}}"#,
    )
    .unwrap();

    store.migrate_split_messages().await.unwrap();
    assert!(dir.join(MESSAGES_FILE).is_file(), "消息应被拆出去");

    // 迁移后清单仍有友好名，且元数据里不再内联消息
    let listed = store.list_sessions().await.unwrap();
    assert_eq!(listed[0].title, "迁移前的话");
    let meta_text = std::fs::read_to_string(dir.join(SESSION_FILE)).unwrap();
    assert!(!meta_text.contains("\"messages\""));

    // 幂等：再跑一次不报错、内容不变
    store.migrate_split_messages().await.unwrap();
    let again = store.list_sessions().await.unwrap();
    assert_eq!(again[0].title, "迁移前的话");
    assert_eq!(again[0].message_count, 1);
}

/// 有界清单：`limit` 条 + `before` 游标（按 `updated_at` 降序之后往前翻）
#[tokio::test]
async fn session_list_pages_before_cursor() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    for (id, t) in [("a", 3000), ("b", 2000), ("c", 1000)] {
        save(&store, &session_with(id, None)).await;
        // session_with 固定 updated_at=1000，这里改成期望值以排出顺序
        let mut s = store.load_session(id).await.unwrap();
        s.updated_at = t;
        save(&store, &s).await;
    }

    let ids = |v: &[SessionSummary]| -> Vec<String> { v.iter().map(|s| s.id.clone()).collect() };

    let page1 = store.list_sessions_window(Some(2), None).await.unwrap();
    assert_eq!(ids(&page1), vec!["a", "b"]);

    let page2 = store
        .list_sessions_window(Some(2), Some("b"))
        .await
        .unwrap();
    assert_eq!(ids(&page2), vec!["c"]);

    // 不传参数 = 全量（与 `list_sessions` 同义）
    assert_eq!(ids(&store.list_sessions().await.unwrap()), vec!["a", "b", "c"]);

    // 游标是最后一页 ⇒ 空页，自然收敛
    assert!(store
        .list_sessions_window(Some(2), Some("c"))
        .await
        .unwrap()
        .is_empty());
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
