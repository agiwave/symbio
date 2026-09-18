//! `plugin/vdfs_provider.rs` 的单元测试（`impl VdfsProvider` 的对外行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
// trait 方法（list/stat/read/write/delete/watch/unwatch）需 trait 在作用域内才可解析
use crate::symbio_core::vdfs_provider::VdfsProvider;
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
    // 使 `.vdfs` 左栏与详情恒等
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
    let listed = session_node(&SessionSummary::of(&s), &SessionRuntime::idle());
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

// ==================== 配置文档（`.vdfs/session/PLUGIN.yml`）的 provider 侧 ====================
//
// 定义侧的测试（`config_definition` 与 `SessionConfig::default()` 同源）在
// `plugin.test.rs`；这里只测**经 provider 访问**的行为。

/// 配置文件是 `ext = form` 的可写文档，**但它不在会话清单里**。
///
/// 「清单 = 业务列表」：`.vdfs/session` 下应当只有会话。配置文件进设置菜单走
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

// ==================== 会话记忆（`.vdfs/session/<id>/AGENTS.md`）====================

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
