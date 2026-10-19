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
    save(&store, &session_with("sess_aaa", None)).await;

    assert!(tmp.path().join("sess_aaa").join("session.json").exists());
    let listed = store.list_sessions_window(None, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "sess_aaa");
}

/// `load_session_checked` 是存在性判据：命中给 `Some`、未命中给 `None`（而不是空
/// 会话），且**按 id 直取**——嵌套子会话同样命中，无需先列全量清单再 `find`
#[tokio::test]
async fn load_session_checked_distinguishes_missing_from_empty() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("sess_parent", None)).await;
    save(&store, &session_with("sess_child", Some("sess_parent"))).await;

    // 命中：顶层与嵌套子会话都凭自身 id 直取
    for id in ["sess_parent", "sess_child"] {
        let got = store.load_session_checked(id).await.unwrap();
        assert_eq!(got.map(|s| s.id), Some(id.to_string()), "{id} 应命中");
    }

    // 未命中：`None`——这正是 `load_session`（返回空会话）无法给出的区分
    assert!(store
        .load_session_checked("sess_none")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.load_session("sess_none").await.unwrap().id,
        "sess_none",
        "同一份存储上 load_session 仍按「未命中 = 空会话」回答"
    );

    // 不落盘型同契约
    let mem = SessionStore::ephemeral();
    assert!(mem
        .load_session_checked("sess_none")
        .await
        .unwrap()
        .is_none());
    save(&mem, &session_with("sess_mem", None)).await;
    assert!(mem
        .load_session_checked("sess_mem")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn sub_session_routes_into_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("sess_parent", None)).await;
    save(&store, &session_with("sess_child", Some("sess_parent"))).await;

    // 嵌套目录布局：<parent>/sessions/<child>/session.json
    assert!(tmp
        .path()
        .join("sess_parent")
        .join("sessions")
        .join("sess_child")
        .join("session.json")
        .exists());

    // 子会话不出现在顶层清单
    let listed = store.list_sessions_window(None, None).await.unwrap();
    assert_eq!(listed.len(), 1);

    // 子会话清单：仅父目录内的嵌套会话
    let subs = store.list_sub_sessions("sess_parent").await.unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, "sess_child");
    assert!(store
        .list_sub_sessions("sess_noparent")
        .await
        .unwrap()
        .is_empty());

    // 子会话可凭自身 id 寻址（load / session_dir / 删除）
    let loaded = store.load_session("sess_child").await.unwrap();
    assert_eq!(loaded.id, "sess_child");
    assert_eq!(
        store.session_dir("sess_child").unwrap(),
        tmp.path()
            .join("sess_parent")
            .join("sessions")
            .join("sess_child")
    );
    store.delete_session("sess_child").await.unwrap();
    assert!(store
        .list_sub_sessions("sess_parent")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn deleting_parent_cascades_sub_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("sess_parent", None)).await;
    save(&store, &session_with("sess_child1", Some("sess_parent"))).await;
    save(&store, &session_with("sess_child2", Some("sess_parent"))).await;

    store.delete_session("sess_parent").await.unwrap();
    assert!(store
        .list_sessions_window(None, None)
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .list_sub_sessions("sess_parent")
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
    assert!(store
        .list_sessions_window(None, None)
        .await
        .unwrap()
        .is_empty());

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
        store.list_sessions_window(None, None).await.unwrap().len(),
        1,
        "mem_a 的两次写不应变成两条"
    );

    // list 按 updated_at 降序（与落盘型同一份排序规则）
    save(&store, &session_with("mem_b", None)).await; // 1000
    let listed = store.list_sessions_window(None, None).await.unwrap();
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
    let listed = store.list_sessions_window(None, None).await.unwrap();
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

    assert_eq!(a.list_sessions_window(None, None).await.unwrap().len(), 1);
    assert!(
        b.list_sessions_window(None, None).await.unwrap().is_empty(),
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
    save(&store, &session_with("sess_heal", None)).await;

    let path = file_for(tmp.path(), "sess_heal");
    let good = std::fs::read_to_string(&path).unwrap();
    // 造一份「完整 JSON 之后又粘了半个对象」的坏文件（trailing characters 形态）
    std::fs::write(&path, format!("{good}{{\"id\":\"residue\"")).unwrap();

    let healed = store.load_session("sess_heal").await.unwrap();
    assert_eq!(healed.id, "sess_heal", "首个完整对象应被读出");

    // 彻底坏掉（首个对象都不完整）时才是解析失败，不能静默当成空会话
    std::fs::write(&path, "{\"id\":\"sess_heal\",\"messages\":[{").unwrap();
    assert!(matches!(
        store.load_session("sess_heal").await.unwrap_err(),
        PluginError::ParseError(_)
    ));
}

/// 原子写的收尾：成功路径上不得留下 `.tmp`（否则残留会随每次保存累积）
#[tokio::test]
async fn save_leaves_no_temp_file_behind() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with("sess_atomic", None)).await;
    save(&store, &session_with("sess_atomic", None)).await;

    let dir = dir_for(tmp.path(), "sess_atomic");
    assert!(dir.join(SESSION_FILE).is_file());
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "rename 后临时文件不该存在");
}

// ==================== 元数据 / 消息分文件（清单性能） ====================
//
// 这一组断言钉住的是「列一次清单的成本与会话聊了多久无关」：清单只读
// `session.json`，`messages.json` 只有真的要取转写时才读。

/// 造一条用户文本消息（标题 / 摘要推导的输入）
fn text_msg(role_is_user: bool, text: &str) -> ChatNode {
    use crate::symbio_core::chat_node::{MessageContent, NodeRole};
    ChatNode {
        id: format!("m{}", text.len()),
        role: Some(if role_is_user {
            NodeRole::User
        } else {
            NodeRole::Assistant
        }),
        content: Some(MessageContent::Text(text.into())),
        ..Default::default()
    }
}

/// 用户格事件（载荷带节点 id；与 `rebuild.test.rs::user_event` 同形——那边是投影
/// 判据、这边是存储判据，两处构造一致才能判到同一件事）
fn user_event(eid: &str, node_id: &str, text: &str) -> crate::symbio_core::Event {
    use crate::symbio_core::{Entity, Event, Verb, EVENT_USER_MESSAGE};
    Event::pending(
        eid,
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        1,
        "user",
    )
    .with_ts(1_000)
    .with_payload(serde_json::json!({ "text": text, "tier": "full", "id": node_id }))
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

/// 保存只写元数据：消息正文不归 store 写（[ADR-049]——`v2-events.wal` 是唯一
/// 事实源，正文由事实路径入格；`messages.json` 停写，v1 遗留文件不再有写方）
#[tokio::test]
async fn save_writes_metadata_only() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with_messages("sess_split", 1000)).await;

    let dir = dir_for(tmp.path(), "sess_split");
    assert!(dir.join(SESSION_FILE).is_file());
    assert!(
        !dir.join(MESSAGES_FILE_NAME).exists(),
        "save 不得再写 messages.json——正文的事实都在 WAL 里"
    );

    let meta_text = std::fs::read_to_string(dir.join(SESSION_FILE)).unwrap();
    assert!(
        !meta_text.contains("\"messages\""),
        "元数据里不该内联消息：{meta_text}"
    );

    // 没有事实（WAL 不在）⇒ 读回空消息：store 层不再持久化正文，这是新契约
    let loaded = store.load_session("sess_split").await.unwrap();
    assert!(loaded.messages.is_empty());
}

/// [ADR-049] 真源判据：有 WAL 时 `messages.json` **缺失 / 写坏**都不影响读出完整
/// 历史——读侧不依赖缓存文件，且**不读**那份坏缓存（坏缓存必须被无视，而不是
/// 「恰好没坏所以没暴露」）。这是 t43 存储手术判断的 Rust 归属。
#[tokio::test]
async fn wal_is_the_source_messages_json_is_ignored() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    // 事实入格走生产同一条 append 路（事件 → `<会话目录>/v2-events.wal`）
    let dir = dir_for(tmp.path(), "sess_truth");
    std::fs::create_dir_all(&dir).unwrap();
    {
        let wal = crate::plugins::session::store::EventWalStore::open(
            dir.join(crate::plugins::session::paths::V2_WAL_FILE),
        )
        .unwrap();
        use crate::symbio_core::Store as _;
        wal.append(user_event("e1", "n1", "第一问")).unwrap();
        wal.append(user_event("e2", "n2", "第二问")).unwrap();
    }
    save(&store, &session_with("sess_truth", None)).await;
    assert!(
        !dir.join(MESSAGES_FILE_NAME).exists(),
        "save 不得写缓存文件"
    );

    // ① 缓存缺席 ⇒ 历史从 WAL 完整重建
    let loaded = store.load_session("sess_truth").await.unwrap();
    assert_eq!(loaded.messages.len(), 2, "WAL 重建应给出两条消息");
    assert_eq!(loaded.messages[0].id, "n1");
    assert_eq!(loaded.messages[1].id, "n2");

    // ② 缓存写坏 ⇒ 仍读 WAL（真源优先的甄别判据：坏缓存被无视）
    std::fs::write(
        dir.join(MESSAGES_FILE_NAME),
        r#"{"messages":[{"id":"evil","role":"user","content":"毒"}]}"#,
    )
    .unwrap();
    let loaded = store.load_session("sess_truth").await.unwrap();
    assert_eq!(loaded.messages.len(), 2, "坏缓存不得改变历史");
    assert!(
        loaded.messages.iter().all(|m| m.id != "evil"),
        "坏缓存的内容不得被读进来"
    );
}

/// [ADR-049] 回落判据：WAL 不在时回落 `messages.json`——v1 遗留会话的事实从未
/// 入格，那个文件是它们历史的唯一记录，回落读法必须还在
#[tokio::test]
async fn legacy_messages_json_reads_back_when_no_wal() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    let dir = dir_for(tmp.path(), "sess_v1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SESSION_FILE),
        r#"{"id":"sess_v1","created_at":1,"updated_at":2,"metadata":{}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join(MESSAGES_FILE_NAME),
        r#"{"messages":[{"id":"m1","role":"user","content":"遗留的话"},{"id":"m2","role":"assistant","content":"收到"}]}"#,
    )
    .unwrap();

    let loaded = store.load_session("sess_v1").await.unwrap();
    assert_eq!(loaded.messages.len(), 2, "v1 遗留会话的历史必须仍读得回来");
}

/// 清单只读元数据：**消息文件坏了也不影响列清单**（这是拆分的核心收益）
#[tokio::test]
async fn list_sessions_does_not_touch_messages() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    save(&store, &session_with_messages("sess_ok", 1000)).await;

    // 把消息文件写成垃圾：清单仍然列得出来
    std::fs::write(
        dir_for(tmp.path(), "sess_ok").join(MESSAGES_FILE_NAME),
        "{ not json",
    )
    .unwrap();

    let listed = store.list_sessions_window(None, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "sess_ok");
    // 投影还在（保存时算好的），因此清单不受影响
    assert_eq!(listed[0].message_count, 2);
}

/// 保存时算好投影：标题 / 条数 / 摘要 / 标签都不必读消息就能拿到
#[tokio::test]
async fn save_persists_the_list_projection() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    let mut s = session_with_messages("sess_proj", 1000);
    s.metadata["workdir"] = json!("/tmp/proj/demo");
    save(&store, &s).await;

    let listed = store.list_sessions_window(None, None).await.unwrap();
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
    let dir = dir_for(tmp.path(), "sess_legacy");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SESSION_FILE),
        r#"{"id":"sess_legacy","messages":[{"id":"m1","role":"user","content":"老会话的标题"},{"id":"m2","role":"assistant","content":"收到"}],"created_at":1,"updated_at":2,"metadata":{}}"#,
    )
    .unwrap();

    let listed = store.list_sessions_window(None, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].title, "老会话的标题",
        "投影缺失时必须用内联消息就地补算，否则清单会显示 id"
    );
    assert_ne!(listed[0].title, listed[0].id);
    assert_eq!(listed[0].message_count, 2);

    // 消息也读得回来（旧布局没有 messages.json）
    let loaded = store.load_session("sess_legacy").await.unwrap();
    assert_eq!(loaded.messages.len(), 2);
}

/// 迁移：旧布局 → 两文件，并补写投影；**幂等**（第二次跑不再改动）
#[tokio::test]
async fn migrate_splits_legacy_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());

    let dir = dir_for(tmp.path(), "sess_mig");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SESSION_FILE),
        r#"{"id":"sess_mig","messages":[{"id":"m1","role":"user","content":"迁移前的话"}],"created_at":1,"updated_at":2,"metadata":{}}"#,
    )
    .unwrap();

    store.migrate_split_messages().await.unwrap();
    assert!(dir.join(MESSAGES_FILE_NAME).is_file(), "消息应被拆出去");

    // 迁移后清单仍有友好名，且元数据里不再内联消息
    let listed = store.list_sessions_window(None, None).await.unwrap();
    assert_eq!(listed[0].title, "迁移前的话");
    let meta_text = std::fs::read_to_string(dir.join(SESSION_FILE)).unwrap();
    assert!(!meta_text.contains("\"messages\""));

    // 幂等：再跑一次不报错、内容不变
    store.migrate_split_messages().await.unwrap();
    let again = store.list_sessions_window(None, None).await.unwrap();
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
    assert_eq!(
        ids(&store.list_sessions_window(None, None).await.unwrap()),
        vec!["a", "b", "c"]
    );

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

/// 原子写的 tmp 名必须**每次唯一**——固定名会让并发保存交错写同一个 tmp，
/// 于是 rename 出来的可能是别人写的内容（rename 只保证不写坏，不保证写对人）。
#[test]
fn atomic_tmp_paths_are_unique_per_call() {
    let target = std::path::PathBuf::from("somewhere").join("messages.json");
    let a = tmp_path_for(&target);
    let b = tmp_path_for(&target);
    assert_ne!(a, b, "两次调用必须给出不同 tmp 名");
    let name = a.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("messages.json."),
        "应保留目标名前缀：{name}"
    );
    assert!(name.ends_with(".tmp"), "{name}");
    assert_eq!(
        a.parent(),
        target.parent(),
        "tmp 必须落在同目录（同盘才能 rename）"
    );
}

/// 同一会话的并发保存不得互相覆盖（「整份 load → 改 → 整份重写」的丢更新）
///
/// 判据载体是**元数据计数**（消息已不归 save 持久化，见 [ADR-049]）：锁仍然必须
/// 包住整段「读 → 改 → 写」，否则 20 次自增会丢。
#[tokio::test]
async fn concurrent_saves_of_one_session_serialize_via_lock() {
    let tmp = TempDir::new().unwrap();
    let store = std::sync::Arc::new(SessionStore::new(tmp.path().to_path_buf()));

    let mut tasks = Vec::new();
    for _ in 0..20 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            // 临界区必须包住整段「读 → 改 → 写」；只锁 save 防不住丢更新
            let _guard = store.lock_writes("sess_conc").await;
            let mut s = store.load_session("sess_conc").await.unwrap();
            let n = s.metadata["counter"].as_u64().unwrap_or(0) + 1;
            s.metadata["counter"] = json!(n);
            store.save_session(&s).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }

    let got = store.load_session("sess_conc").await.unwrap();
    assert_eq!(
        got.metadata["counter"],
        json!(20),
        "每次自增都必须留下（无锁时会丢更新）"
    );
}

/// **现行窗口 = 从最近一份快照起**（含快照本身）；没有快照 ⇒ 整段都是现行。
///
/// 这条钉的是归一的前提（[plan/14 §2.2](../../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 推论 1）：
/// 读口只给**完整史**这一种口径，而「这一轮还开着没收束」这类判据要窗口——窗口由
/// 消费方按**事实**切（最近一份快照在哪），不是存储层再开一个读法。
#[test]
fn current_window_starts_at_the_latest_snapshot() {
    use crate::symbio_core::{
        chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus},
        Entity, Event, Verb, EVENT_SESSION_CONSOLIDATED, EVENT_USER_MESSAGE,
    };
    let user = |id: &str, turn: u64| {
        Event::pending(
            id,
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            turn,
            "session/main",
        )
    };
    let snap = |id: &str| {
        Event::pending(
            id,
            EVENT_SESSION_CONSOLIDATED,
            Entity::Message,
            Verb::Progressed,
            0,
            "session/main",
        )
        .with_payload(json!({
            "messages": [ChatNode {
                id: format!("{id}-body"),
                msg_type: Some(NodeKind::Body),
                content: Some(MessageContent::Text("[CONTEXT SNAPSHOT]".into())),
                status: Some(NodeStatus::Completed),
                ..Default::default()
            }],
            "turns": 1,
        }))
    };

    let events = vec![
        user("u0", 0),
        snap("s1"),
        user("u1", 1),
        snap("s2"),
        user("u2", 2),
    ];
    let window = current_window(&events);
    assert_eq!(window[0].event_id, "s2", "窗口起点就是那份基线本身");
    assert_eq!(window.len(), 2, "它 + 其后那一条（更早的一律不再参与）");

    // 没压过 ⇒ 整段都是现行——窗口不是「裁掉前 N 条」那种默认裁剪。
    let plain = vec![user("u0", 0), user("u1", 1)];
    assert_eq!(current_window(&plain).len(), 2);

    // 坏载荷的那份**不算**快照：跳过它继续往前找（与 `mutations_latest_snapshot`
    // 「一条坏载荷不该吃掉此前有效那一份」同一条容错方向）。
    let broken = Event::pending(
        "s3",
        EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        0,
        "session/main",
    )
    .with_payload(json!({ "messages": [], "turns": 1 }));
    let events = vec![user("u0", 0), snap("s1"), user("u1", 1), broken];
    assert_eq!(
        current_window(&events)[0].event_id,
        "s1",
        "空列表不是一份快照 ⇒ 基线仍是上一份，前缀不被它抹掉"
    );
}
