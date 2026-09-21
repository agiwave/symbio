//! `plugin/vdfs_provider.rs` 的单元测试（`impl VdfsProvider` 的对外行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
// trait 方法（list/stat/read/write/delete/watch/unwatch）需 trait 在作用域内才可解析
use crate::symbio_core::vdfs_provider::VdfsProvider;
// 地址构造辅助：只被本测试用，故不经 `plugin.rs` 的共享面转出（那里会让
// `unused_imports` 误报——它看不见「仅经 glob 链使用」的再导出）。
use crate::plugins::session::plugin::nodes::{message_dir_path, message_path};
// 每例独占存储根；guard 在插件之后释放，失败时也会清理。
fn fixture() -> (tempfile::TempDir, SessionPlugin) {
    let dir = tempfile::tempdir().unwrap();
    let plugin = SessionPlugin::new(
        None,
        SessionConfig::default(),
        crate::symbio_core::PluginDir::at(dir.path(), "session"),
    );
    (dir, plugin)
}

// ==================== VDFS provider ====================

fn vctx() -> vdfs::VdfsContext {
    vdfs::VdfsContext::empty()
}

/// provider 自描述：**不含挂载名**——挂载名由使用方在注册时选定
/// （见 `traverse` 里的 `register_vdfs_provider(PLUGIN_SESSION, ..)`）
#[tokio::test]
async fn vdfs_self_description_has_no_mount() {
    let (_dir, p) = fixture();
    assert_eq!(p.label(), Some("会话"));
    assert_eq!(p.icon(), Some("session"));
    // 顺序由本 provider 的 order() 自持（**单一真相源**），
    // 使 `<根>` 左栏与详情恒等
    assert_eq!(p.order(), 1);
    assert_eq!(p.root_access().flags(), "l");
    assert!(!p.root_access().traverse, "会话是叶子，不参与树遍历");

    // 根下可新建「会话」——类型清单即「新建」入口的唯一依据
    let types = p.root_new_types();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].ext, vdfs::VDFS_EXT_SESSION);
    assert_eq!(types[0].title, "会话");
}

// 根目录已隔离，id 只需在本例内稳定。
fn unique_id(tag: &str) -> String {
    tag.to_owned()
}

/// 不存在的会话：list / stat 一律 NotFound（不做静默降级）
#[tokio::test]
async fn vdfs_list_unknown_session_is_not_found() {
    let (_dir, p) = fixture();
    let id = unique_id("no-such-session");
    assert!(p.list(&vctx(), &id).await.is_err());
    assert!(p.stat(&vctx(), &id).await.is_err());
}

/// `<id>` 的 `stat` 是**目录视图**（只给 `l`），但呈现必须与清单同源。
///
/// `status` 是运行态的唯一取值点：变更通知 → 重读 `stat` → `status == working`。
/// 少了它，前端刚乐观置上的「运行中」会被下一次变更立刻改回空闲。
#[tokio::test]
async fn vdfs_stat_session_is_dir_view_with_list_shape() {
    let (_dir, p) = fixture();
    let id = unique_id("stat-view");
    let mut s = Session::new(&id);
    s.updated_at = 1_700_000_000;
    p.save_session(&s).await.unwrap();

    let n = p.stat(&vctx(), &id).await.unwrap();
    assert_eq!(n.name, id);
    assert_eq!(n.access.flags(), "l", "目录视图：可列，但不给新建入口");
    assert!(n.is_dir(), "被当目录访问时的视图");

    // 同源判据：与清单节点逐字段一致（不是另写一份「目录版」形状）
    let listed = session_node(&SessionSummary::of(&s), &SessionRuntime::idle(None));
    assert_eq!(n.status, listed.status);
    assert_eq!(n.ext, listed.ext);
    assert_eq!(n.updated_at, listed.updated_at);
    assert_eq!(
        n.status,
        vdfs::VDFS_STATUS_ACTIVE,
        "空闲是显式状态值，不是空串"
    );
}

/// 实时：`watch` 登记的 sink 在 `notify_change` 时**同步**收到变更；
/// `unwatch` 按引用计数摘除（严格配对，归零才真正停投）
#[tokio::test]
async fn vdfs_watch_forwards_session_changes() {
    let (_dir, p) = fixture();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<vdfs::VdfsChange>();
    let sink: vdfs::VdfsChangeSink = Arc::new(move |c| {
        let _ = tx.send(c);
    });

    p.watch(&vctx(), "", sink.clone()).await.unwrap();
    p.watch(&vctx(), "", sink).await.unwrap();
    assert_eq!(p.change_subs.subscriber_count(), 2);
    assert_eq!(p.change_subs.paths(), vec!["".to_string()]);

    p.notify_change("abc", vdfs::VDFS_CHANGE_CREATED);

    // 投递是同步的，但两条订阅共用一个投递器 ⇒ 恰好一次
    let got = rx.recv().await.expect("变更应经 watch 投递到 sink");
    assert_eq!(got.path, "abc");
    assert_eq!(got.change, vdfs::VDFS_CHANGE_CREATED);
    assert!(rx.try_recv().is_err(), "同一路径的重复订阅不得收到重复帧");

    p.unwatch(&vctx(), "").await.unwrap();
    assert_eq!(
        p.change_subs.subscriber_count(),
        1,
        "取消一位仍有另一位，不得提前停投"
    );
    p.unwatch(&vctx(), "").await.unwrap();
    assert!(
        !p.change_subs.has_subscribers(),
        "unwatch 必须把该路径彻底摘掉"
    );
    p.notify_change("abc", vdfs::VDFS_CHANGE_CREATED);
    assert!(rx.try_recv().is_err(), "无订阅者时不得投递");
}

// ==================== 配置文档（`<根>/session/PLUGIN.yml`）的 provider 侧 ====================
//
// 定义侧的测试（`config_definition` 与 `SessionConfig::default()` 同源）在
// `plugin.test.rs`；这里只测**经 provider 访问**的行为。

/// 配置文件是 `ext = form` 的可写文档，**但它不在会话清单里**。
///
/// 「清单 = 业务列表」：`<根>/session` 下应当只有会话。配置文件进设置菜单走
/// 的是 ConfigurableVisitor 那条通道（`announce_configurable`），不靠清单并列
/// ——否则列表底部会多出一个「设置」项。可达性不受影响：`stat` / `read` 照常。
#[tokio::test]
async fn config_document_is_reachable_but_not_a_session_list_item() {
    let (_dir, p) = fixture();

    // 可达：按真实文件名 stat / read
    let node = p.stat(&vctx(), PLUGIN_FILE).await.unwrap();
    assert_eq!(node.name, PLUGIN_FILE, "地址就是插件目录里的真实文件名");
    assert_eq!(node.ext.as_deref(), Some(vdfs::VDFS_EXT_FORM));
    assert_eq!(node.access.flags(), "rw");
    assert!(node.schema.is_some(), "定义随节点下发");

    // 不并列：清单里没有它
    let items = p.list(&vctx(), "").await.unwrap();
    assert!(
        !items.iter().any(|n| n.name == PLUGIN_FILE),
        "会话清单里只应有会话，配置文件不该出现"
    );

    // 配置文件不是会话 id：按文件读，不按会话解析
    assert_eq!(
        p.stat(&vctx(), PLUGIN_FILE).await.unwrap().name,
        PLUGIN_FILE
    );
    let content = p.read(&vctx(), PLUGIN_FILE).await.unwrap();
    let cfg: SessionConfig = serde_json::from_str(content.text.as_deref().unwrap()).unwrap();
    assert_eq!(cfg.max_messages, SessionConfig::default().max_messages);

    // 文档没有子项，也不可删除
    assert!(p.list(&vctx(), PLUGIN_FILE).await.is_err());
    assert!(p.delete(&vctx(), PLUGIN_FILE, false).await.is_err());
}

/// 配置写入：校验先于一切（字段级错误），坏值不会改动内存
#[tokio::test]
async fn config_write_validates_before_applying() {
    let (_dir, p) = fixture();
    let before = p.config.read().await.max_messages;
    let bad = vdfs::VdfsContent::text("", r#"{"max_messages": 1}"#);
    match p.write(&vctx(), PLUGIN_FILE, &bad).await {
        Err(vdfs::VdfsError::Invalid(v)) => assert_eq!(v.fields[0].field, "max_messages"),
        other => panic!("应为字段级校验错误，实得 {other:?}"),
    }
    assert_eq!(p.config.read().await.max_messages, before);
}

// ==================== 会话记忆（`<根>/session/<id>/AGENTS.md`）====================

/// 会话记忆是会话内部的一个**可读写文件**：与三个目录并列、`stat` 与 `list` 同源、
/// 写后读得回、**不可删除**（与 work / agent 的记忆层同一内核约定）。
#[tokio::test]
async fn memory_is_a_read_write_file_inside_the_session() {
    let (_dir, p) = fixture();
    let id = unique_id("memory");
    p.save_session(&Session::new(&id)).await.unwrap();
    let path = format!("{id}/{}", crate::symbio_core::AGENTS_FILE);

    // ① 会话内部并列着记忆（它本来就是会话的一部分，不另开一条寻址）
    let items = p.list(&vctx(), &id).await.unwrap();
    let mem = items
        .iter()
        .find(|n| n.name == crate::symbio_core::AGENTS_FILE)
        .expect("会话内部应列出记忆文件");
    assert!(!mem.is_dir(), "记忆是文件，不是目录");
    assert_eq!(mem.access.flags(), "rw", "模型与用户共用这一份，可读可写");
    assert_eq!(mem.kind, PLUGIN_SESSION);
    assert!(mem.description.is_some(), "列表里要能看出它是干什么的");

    // ② `stat` 与 `list` 同源（同一份形状，不是另写一份「详情版」）
    let stat = p.stat(&vctx(), &path).await.unwrap();
    assert_eq!(stat.name, mem.name);
    assert_eq!(stat.access.flags(), mem.access.flags());
    assert_eq!(stat.size, mem.size);
    assert_eq!(stat.description, mem.description);

    // ③ 还没写过 → 空串（「还没写过」是记忆的正常状态，不是错误）
    assert_eq!(
        p.read(&vctx(), &path).await.unwrap().text.as_deref(),
        Some("")
    );

    // ④ 写入 → 读回（写闸门在内核，本层不重复实现）
    let r = p
        .write(
            &vctx(),
            &path,
            &vdfs::VdfsContent::text("", "本会话约定：所有时间用 UTC。"),
        )
        .await
        .unwrap();
    assert!(r.created, "首次写入应报 created");
    assert_eq!(
        p.read(&vctx(), &path).await.unwrap().text.as_deref(),
        Some("本会话约定：所有时间用 UTC。")
    );
    let again = p
        .write(&vctx(), &path, &vdfs::VdfsContent::text("", "改主意了"))
        .await
        .unwrap();
    assert!(!again.created, "覆盖写入不是 created");

    // ⑤ 记忆不可删除：要清空就写空内容（一次可读、可审、可撤销的显式动作）
    assert!(
        matches!(
            p.delete(&vctx(), &path, false).await,
            Err(vdfs::VdfsError::Forbidden(_))
        ),
        "删除即丢失本会话的长期约定，必须明确拒绝"
    );
    assert!(p.read(&vctx(), &path).await.is_ok(), "拒绝删除后内容仍在");

    // ⑥ 记忆是文件：没有子项；更深层级也不解析
    assert!(p.list(&vctx(), &path).await.is_err());
}

/// 写入闸门取自 `SessionConfig`：配小 → 同一个写入被**拒绝**（而不是截断），
/// 且被拒绝的写入不得留下半截内容。
#[tokio::test]
async fn memory_write_respects_the_configured_gate() {
    let (_dir, p) = fixture();
    *p.config.write().await = SessionConfig {
        memory_max_bytes: 4,
        ..SessionConfig::default()
    };
    let id = unique_id("memory-gate");
    p.save_session(&Session::new(&id)).await.unwrap();
    let path = format!("{id}/{}", crate::symbio_core::AGENTS_FILE);

    assert!(
        p.write(&vctx(), &path, &vdfs::VdfsContent::text("", "12345"))
            .await
            .is_err(),
        "超出写入上限必须被拒绝"
    );
    assert_eq!(
        p.read(&vctx(), &path).await.unwrap().text.as_deref(),
        Some(""),
        "被拒绝的写入不得留下半截内容"
    );
}

/// 不存在的会话：记忆路径一律 NotFound（不为幽灵会话造一份记忆）
#[tokio::test]
async fn memory_of_unknown_session_is_not_found() {
    let (_dir, p) = fixture();
    let id = unique_id("memory-ghost");
    let path = format!("{id}/{}", crate::symbio_core::AGENTS_FILE);

    assert!(p.stat(&vctx(), &path).await.is_err());
    assert!(p.read(&vctx(), &path).await.is_err());
    assert!(p
        .write(&vctx(), &path, &vdfs::VdfsContent::text("", "x"))
        .await
        .is_err());
}

/// 记忆写入经订阅表投递变更（与 `list` 的节点地址同一坐标系：`<id>/AGENTS.md`）
#[tokio::test]
async fn memory_write_notifies_subscribers() {
    let (_dir, p) = fixture();
    let id = unique_id("memory-notify");
    p.save_session(&Session::new(&id)).await.unwrap();
    let path = format!("{id}/{}", crate::symbio_core::AGENTS_FILE);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<vdfs::VdfsChange>();
    let sink: vdfs::VdfsChangeSink = Arc::new(move |c| {
        let _ = tx.send(c);
    });
    p.watch(&vctx(), &id, sink).await.unwrap();

    p.write(&vctx(), &path, &vdfs::VdfsContent::text("", "记一笔"))
        .await
        .unwrap();

    let got = rx.recv().await.expect("写入应投递一条变更");
    assert_eq!(
        got.path,
        format!("{id}/{}", crate::symbio_core::AGENTS_FILE),
        "变更路径与 list 返回的节点地址同源"
    );
    assert_eq!(got.change, vdfs::VDFS_CHANGE_CREATED, "首次写入是 created");
}

// ==================== 会话 id 形状 ====================

/// 新建会话的 id 是**短 GUID**（8 位十六进制）。
///
/// 会话 id 直接出现在用户视野里——它是 VDFS 目录名（`.symbio/session/<id>`），
/// 列表与地址栏都要读。此前是 36 字符带连字符的完整 UUID，与项目既有的两处
/// 短 id 约定（`turn::short_id` / `vdfs_service::entry::auto_id`）不一致。
///
/// 这条测试盯的是**形状**：一旦有人改回长 UUID，这里立刻红。
#[tokio::test]
async fn new_session_id_is_a_short_guid() {
    let (_dir, p) = fixture();
    let content = vdfs::VdfsContent {
        create: true,
        ..vdfs::VdfsContent::text("", "{}")
    };
    let r = p.write(&vctx(), "", &content).await.unwrap();
    assert!(r.created);

    let id = r.path;
    assert_eq!(id.len(), 8, "短 GUID 应为 8 位，实得 {id:?}");
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "应为十六进制字符，实得 {id:?}"
    );
    assert!(!id.contains('-'), "短 GUID 不带连字符，实得 {id:?}");

    // 生成后确实落了盘：能按这个 id 读回会话
    assert!(
        p.get_store().await.unwrap().session_dir(&id).is_some(),
        "新会话目录应按 id 建出"
    );
}

/// 连续新建不会撞 id（碰撞即新建失败，属于用户可见错误）。
#[tokio::test]
async fn new_session_ids_are_distinct() {
    let (_dir, p) = fixture();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..32 {
        let r = p
            .write(
                &vctx(),
                "",
                &vdfs::VdfsContent {
                    create: true,
                    ..vdfs::VdfsContent::text("", "{}")
                },
            )
            .await
            .unwrap();
        assert!(ids.insert(r.path.clone()), "id 重复：{}", r.path);
    }
    assert_eq!(ids.len(), 32);
}

// ==================== 转写区段（`<根>/session/<id>/消息`）的三个入口 ====================
//
// 消息的**改写 / 截断 / 清空**曾经各有专用路由（`chat/update_message` /
// `chat/delete_message` / `chat/clear_messages`），2026-09-18 迁到 VDFS：
//
// | 操作 | 入口 | 落到转写流上的变更 |
// |---|---|---|
// | 改写某条 | `write(<id>/消息/<mid>)` | 该消息一条 `upsert`（整条替换） |
// | 删该条及其后 | `action(<id>/消息/<mid>, "truncate")` | 一条 `reset` + 回执带被删 id |
// | 清空历史 | `action(<id>/消息, "clear")` | 一条 `reset` |
//
// 「变更」这一列说的是 `session/stream` 的 `NodeOp`（消息变更的**唯一**通道，
// 见 `symbio_core::transcript_stream`），**不是** VDFS 变更：消息域已不往 VDFS
// 变更面发任何东西，旧的三条变更形状（消息上 `updated` / 起始消息上 `truncated` /
// 列表目录上 `deleted`）随之不存在。断言面因此从"VDFS 变更值"换成"转写流的帧"
// （`subscribe_stream` / `drain_stream_frames`）：截断与清空三例各自钉住
// 「发了几帧、是哪一种操作」——这正是新机制真正要守的边界（区间删除**一条**帧，
// 不是 N 条；什么都没删**一条都不发**）。
//
// 本段锁定这三条路径的**对外行为**，并盯住三条不该被打破的边界：
// `create` 意图（新增消息 = 发言，入口只有聊天协议）、`delete`（逐节点语义，
// 表达不了这两种集合操作）、以及「什么都没删 ⇒ 不发变更」。

/// 造一个带 N 条消息的会话：id 为 `m0..mN`、`seq` 单调（顺序的唯一权威锚点）。
async fn seed_messages(p: &SessionPlugin, id: &str, texts: &[&str]) {
    let mut s = Session::new(id);
    s.messages = texts
        .iter()
        .enumerate()
        .map(|(i, t)| cm::ChatMessage {
            id: format!("m{i}"),
            role: Some(cm::MessageRole::User),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text((*t).to_string())),
            seq: Some(i as i64),
            ..Default::default()
        })
        .collect();
    p.save_session(&s).await.unwrap();
}

/// 转写列表里现有哪几条（经 VDFS 的 `消息` 列表，即使用方看到的那一份）。
async fn transcript_ids(p: &SessionPlugin, id: &str) -> Vec<String> {
    p.list(&vctx(), &message_dir_path(id))
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect()
}

/// 挂一条转写流（`session/stream`）订阅，返回连接 id 与接收端。
///
/// 订阅表是**进程级**的（`transcript_stream::STREAM_SUBS`），而用例并行跑：
/// - 连接 id 必须**全进程唯一**——`unique_id` 是恒等的，拿它拼连接 id 会让并行
///   用例互相覆盖订阅（后注册者赢，前者一帧都收不到）；
/// - 因此断言一律按 `session_id` 过滤：本通道会收到同进程其它用例发布的帧。
fn subscribe_stream() -> (
    String,
    tokio::sync::mpsc::Receiver<crate::symbio_core::PluginFrame>,
) {
    let conn = format!("test-{}", uuid::Uuid::new_v4());
    // 容量给足：本用例最多一帧，溢出的唯一含义是"通道满" → 触发 resync 摘除。
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    crate::symbio_core::transcript_stream::register_transcript_subscriber(conn.clone(), tx);
    (conn, rx)
}

/// 取走该会话**已到达**的转写帧（非本会话的忽略：订阅表是共享的）。
fn drain_stream_frames(
    rx: &mut tokio::sync::mpsc::Receiver<crate::symbio_core::PluginFrame>,
    id: &str,
) -> Vec<serde_json::Value> {
    let mut got = Vec::new();
    while let Ok(crate::symbio_core::PluginFrame::Data(v)) = rx.try_recv() {
        let owned = v
            .get("data")
            .and_then(|d| d.get("session_id"))
            .and_then(|s| s.as_str());
        if owned == Some(id) {
            got.push(v);
        }
    }
    got
}

/// 改写单条消息：只覆盖补丁里**提供**的字段，未提供的原样保留。
///
/// 补丁语义与旧 `chat/update_message` 逐字一致——那次迁移是**搬移**不是重写，
/// 这条测试就是搬移的回归锚。
#[tokio::test]
async fn message_write_patches_only_the_provided_fields() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-patch");
    seed_messages(&p, &id, &["第一条", "第二条"]).await;

    let r = p
        .write(
            &vctx(),
            &message_path(&id, "m0"),
            &vdfs::VdfsContent::text("", r#"{"content":"改过的第一条"}"#),
        )
        .await
        .unwrap();
    assert!(!r.created, "改写既有消息不是新建");

    assert_eq!(
        p.read(&vctx(), &message_path(&id, "m0"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("改过的第一条"),
        "改写后的正文应读得回"
    );

    let msgs = p.transcript_of(&id).await.unwrap();
    let m0 = msgs.iter().find(|m| m.id == "m0").unwrap();
    let m1 = msgs.iter().find(|m| m.id == "m1").unwrap();
    assert_eq!(
        m0.role,
        Some(cm::MessageRole::User),
        "未提供的 role 必须原样保留"
    );
    // 改写走 `replace_messages`，而它会按**数组顺序**重新分配 seq
    // （注释：「数组顺序即权威顺序」）。所以这里锁的是**相对顺序**而非绝对值——
    // 哪天改成"就地更新、不重排"，这条断言依然成立。
    assert!(
        m0.seq < m1.seq,
        "改写后 m0 仍应排在 m1 之前（实得 {:?} / {:?}）",
        m0.seq,
        m1.seq
    );
    assert_eq!(
        transcript_ids(&p, &id).await,
        vec!["m0", "m1"],
        "改写不增删条目"
    );
}

/// 改写**不存在**的消息：报错，不静默新建一条。
#[tokio::test]
async fn message_write_unknown_message_is_not_found() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-patch-ghost");
    seed_messages(&p, &id, &["唯一一条"]).await;

    assert!(
        p.write(
            &vctx(),
            &message_path(&id, "nope"),
            &vdfs::VdfsContent::text("", r#"{"content":"x"}"#),
        )
        .await
        .is_err(),
        "改写不存在的消息必须报错"
    );
    assert_eq!(
        transcript_ids(&p, &id).await,
        vec!["m0"],
        "失败的改写不得改动列表"
    );
}

/// 补丁里的 `id` 与地址不符 → 报错。
///
/// 地址是消息身份的**唯一权威**。静默按地址写下去，会把调用方「发错了节点」
/// 这个它自己并不知道的 bug 埋掉。
#[tokio::test]
async fn message_write_rejects_id_that_contradicts_the_address() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-patch-idclash");
    seed_messages(&p, &id, &["第一条", "第二条"]).await;

    // 两种写法都不得通过：给了**别的** id、给了**空** id
    // （「不给 id」是合法的——见下一例；给了却是错的那种才该报错）
    for patch in [r#"{"id":"m1","content":"想改 m1"}"#, r#"{"id":""}"#] {
        assert!(
            p.write(
                &vctx(),
                &message_path(&id, "m0"),
                &vdfs::VdfsContent::text("", patch),
            )
            .await
            .is_err(),
            "{patch} 的 id 与地址不符，必须报错"
        );
    }
    assert_eq!(
        p.read(&vctx(), &message_path(&id, "m1"))
            .await
            .unwrap()
            .text
            .as_deref(),
        Some("第二条"),
        "冲突的补丁不得落到任何一条上"
    );
}

/// 补丁**不带** `id` 是合法用法：地址就是消息身份，调用方不必把地址里已有的
/// 信息再抄一遍。
///
/// 这条曾经是坏的——`ChatMessage::id` 在结构里是必填字段（它同时是存储层的主键），
/// 于是 `{"content":"…"}` 会在**反序列化阶段**被拒，而「补丁是字段子集、未提供的
/// 保持不变」这条承诺在 `id` 上就是假的。修法是让地址把 id 补齐（地址本就是身份
/// 的唯一权威），而不是要求调用方重复它。
#[tokio::test]
async fn message_write_accepts_a_patch_without_id() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-patch-noid");
    seed_messages(&p, &id, &["第一条"]).await;

    p.write(
        &vctx(),
        &message_path(&id, "m0"),
        &vdfs::VdfsContent::text("", r#"{"status":"completed"}"#),
    )
    .await
    .expect("不带 id 的补丁应按地址补齐，而不是报 JSON 缺字段");

    let msgs = p.transcript_of(&id).await.unwrap();
    assert_eq!(msgs[0].status, Some(cm::MessageStatus::Completed));
    assert_eq!(msgs[0].id, "m0", "id 由地址给出");
    assert_eq!(
        msgs[0].content.as_ref().map(|c| c.to_text()).as_deref(),
        Some("第一条"),
        "未提供的 content 原样保留"
    );
}

/// `create` 意图在消息路径上**一律拒绝**：新增消息就是发言（一整轮编排），
/// 静默接受会把「追加消息不经 VDFS」这条不变量悄悄破掉。
#[tokio::test]
async fn message_write_rejects_create_intent() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-create");
    seed_messages(&p, &id, &["第一条"]).await;

    let content = vdfs::VdfsContent {
        create: true,
        ..vdfs::VdfsContent::text("", r#"{"content":"新消息"}"#)
    };
    assert!(
        p.write(&vctx(), &message_path(&id, "m9"), &content)
            .await
            .is_err(),
        "新建消息即发言，必须走聊天协议"
    );
    assert_eq!(transcript_ids(&p, &id).await, vec!["m0"]);
}

/// 转写**列表**本身不可写：往里放一条 = 发言。
#[tokio::test]
async fn transcript_list_is_not_writable() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-list-ro");
    seed_messages(&p, &id, &["第一条"]).await;

    assert!(
        matches!(
            p.write(
                &vctx(),
                &message_dir_path(&id),
                &vdfs::VdfsContent::text("", r#"{"content":"新消息"}"#)
            )
            .await,
            Err(vdfs::VdfsError::Forbidden(_))
        ),
        "往列表里放一条是发言，不是写入"
    );
}

/// 转写区段不可 `delete`（列表与单条都是）：`delete` 是逐节点语义，
/// 表达不了这两种集合操作。
#[tokio::test]
async fn transcript_segment_is_not_deletable() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-no-delete");
    seed_messages(&p, &id, &["第一条", "第二条"]).await;

    for path in [message_dir_path(&id), message_path(&id, "m0")] {
        let err = p.delete(&vctx(), &path, false).await.unwrap_err();
        assert!(matches!(err, vdfs::VdfsError::Forbidden(_)), "{path}");
    }
    assert_eq!(
        transcript_ids(&p, &id).await,
        vec!["m0", "m1"],
        "被拒的删除不得改动列表"
    );
}

/// 截断：从目标起**及其之后**全部没了；回执给出权威的被删 id 列表。
///
/// 回执是这条路径选择 `action` 而非 `delete` 的关键收益——`vdfs/delete` 只回
/// `{path}`，带不回这个列表，而前端 store 靠它做幂等对齐。
#[tokio::test]
async fn truncate_removes_the_target_and_everything_after() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-truncate");
    seed_messages(&p, &id, &["一", "二", "三", "四"]).await;
    let (conn, mut rx) = subscribe_stream();

    let r = p
        .action(
            &vctx(),
            &message_path(&id, "m1"),
            vdfs::VDFS_ACTION_TRUNCATE,
            None,
        )
        .await
        .unwrap();
    assert!(r.ok);
    assert_eq!(
        r.data,
        Some(json!(["m1", "m2", "m3"])),
        "回执是权威的被删列表"
    );
    assert_eq!(
        transcript_ids(&p, &id).await,
        vec!["m0"],
        "只剩目标之前的那条"
    );

    let frames = drain_stream_frames(&mut rx, &id);
    assert_eq!(frames.len(), 1, "区间删除用**一条**帧表达，不是 N 条");
    assert_eq!(
        frames[0]["data"]["op"].as_str(),
        Some("reset"),
        "区间删除＝让消费端把本地转写整份重读"
    );
    crate::symbio_core::transcript_stream::unregister_transcript_subscriber(&conn);
}

/// 截断一个**不存在**的目标：回执是空列表，列表一条不动，且**一条帧都不发**。
///
/// 「什么都没删」是**结果**不是错误，也不该在转写上留下痕迹——一条 `reset` 会让
/// 消费端把本地转写整份作废重读。
#[tokio::test]
async fn truncate_of_missing_target_changes_nothing() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-truncate-miss");
    seed_messages(&p, &id, &["一", "二"]).await;
    let (conn, mut rx) = subscribe_stream();

    let r = p
        .action(
            &vctx(),
            &message_path(&id, "nope"),
            vdfs::VDFS_ACTION_TRUNCATE,
            None,
        )
        .await
        .unwrap();
    assert!(r.ok, "目标不存在是**结果**，不是错误");
    assert_eq!(r.data, Some(json!([])), "回执是空列表");
    assert_eq!(transcript_ids(&p, &id).await, vec!["m0", "m1"], "列表未动");
    assert!(
        drain_stream_frames(&mut rx, &id).is_empty(),
        "什么都没删 ⇒ 一条帧都不发"
    );
    crate::symbio_core::transcript_stream::unregister_transcript_subscriber(&conn);
}

/// 清空：消息全没了，**会话本体保留**（id / metadata / 工作目录）。
#[tokio::test]
async fn clear_empties_the_transcript_but_keeps_the_session() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-clear");
    seed_messages(&p, &id, &["一", "二", "三"]).await;
    let (conn, mut rx) = subscribe_stream();

    let r = p
        .action(
            &vctx(),
            &message_dir_path(&id),
            vdfs::VDFS_ACTION_CLEAR,
            None,
        )
        .await
        .unwrap();
    assert!(r.ok);
    assert!(transcript_ids(&p, &id).await.is_empty(), "列表应清空");

    let frames = drain_stream_frames(&mut rx, &id);
    assert_eq!(frames.len(), 1, "清空用**一条**帧表达，不是逐条 remove");
    assert_eq!(frames[0]["data"]["op"].as_str(), Some("reset"));
    crate::symbio_core::transcript_stream::unregister_transcript_subscriber(&conn);

    // 会话本体还在（清空不是删除会话）
    let n = p.stat(&vctx(), &id).await.unwrap();
    assert_eq!(n.name, id);
}

/// 动作不认识、或动作放错地址：`NotImplemented`（消费方据此**不给出入口**），
/// 而不是「成功但什么都没做」。
#[tokio::test]
async fn unknown_or_misplaced_action_is_not_implemented() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-action-unknown");
    seed_messages(&p, &id, &["一"]).await;

    for (path, action) in [
        // 动作标识不认识
        (message_path(&id, "m0"), "explode"),
        // 动作对、地址不对：`clear` 落在单条消息上
        (message_path(&id, "m0"), vdfs::VDFS_ACTION_CLEAR),
        // 动作对、地址不对：`truncate` 落在列表目录上
        (message_dir_path(&id), vdfs::VDFS_ACTION_TRUNCATE),
    ] {
        assert!(
            matches!(
                p.action(&vctx(), &path, action, None).await,
                Err(vdfs::VdfsError::NotImplemented)
            ),
            "{path} + {action} 应报 NotImplemented"
        );
    }
    assert_eq!(
        transcript_ids(&p, &id).await,
        vec!["m0"],
        "未实现的动作不得改动列表"
    );
}
