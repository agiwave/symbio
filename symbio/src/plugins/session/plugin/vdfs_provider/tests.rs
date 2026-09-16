//! `plugin/vdfs_provider.rs` 的单元测试（`impl VdfsProvider` 的对外行为）。
//!
//! 与实现同目录分文件（约定同 `store/tests.rs`）：测试跟着被测试的实现走。

use super::*;
// trait 方法（list/stat/read/write/delete/watch/unwatch）需 trait 在作用域内才可解析
use crate::symbio_core::vdfs_provider::VdfsProvider;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::plugins::session::test_dir;

// ==================== VDFS provider ====================

fn vctx() -> vdfs::VdfsContext {
    vdfs::VdfsContext::empty()
}

/// provider 自描述：**不含挂载名**——挂载名由使用方在注册时选定
/// （见 `traverse` 里的 `register_vdfs_provider(PLUGIN_SESSION, ..)`）
#[tokio::test]
async fn vdfs_self_description_has_no_mount() {
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());
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

/// 测试用会话 id：**每个用例唯一**。
///
/// 会话存储目录取自全局 homedir（`<homedir>/plugins/session`）——单元测试
/// 不隔离它（`test_dir()` 只重定向配置文件，不重定向 store）。于是写过
/// 会话的用例会在真实目录里留下文件，下一个复用同一 id 的用例就读到了
/// 别人的数据（`list` 因此不再 NotFound，且结果随并行调度顺序漂移）。
/// 这里给每个用例一个进程内只出现一次的 id，从根上消除串扰。
fn unique_id(tag: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{tag}-{}-{n}", std::process::id())
}

/// 删掉测试自己造的会话目录（不留残留给后续运行）
fn cleanup_session_dir(id: &str) {
    let _ = std::fs::remove_dir_all(SessionPlugin::session_storage_dir().join(id));
}

/// 不存在的会话：list / stat 一律 NotFound（不做静默降级）
#[tokio::test]
async fn vdfs_list_unknown_session_is_not_found() {
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());
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
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());
    let id = unique_id("stat-view");
    let mut s = Session::new(&id);
    s.updated_at = 1_700_000_000;
    p.save_session(&s).await.unwrap();

    let n = p.stat(&vctx(), &id).await.unwrap();
    assert_eq!(n.name, id);
    assert_eq!(n.access.flags(), "l", "目录视图：可列，但不给新建入口");
    assert!(n.is_dir(), "被当目录访问时的视图");

    // 同源判据：与清单节点逐字段一致（不是另写一份「目录版」形状）
    let listed = session_node(&SessionSummary::of(&s), false);
    assert_eq!(n.status, listed.status);
    assert_eq!(n.ext, listed.ext);
    assert_eq!(n.updated_at, listed.updated_at);
    assert_eq!(
        n.status,
        vdfs::VDFS_STATUS_ACTIVE,
        "空闲是显式状态值，不是空串"
    );

    cleanup_session_dir(&id);
}

/// 实时：`watch` 登记的 sink 在 `notify_change` 时**同步**收到变更；
/// `unwatch` 按引用计数摘除（严格配对，归零才真正停投）
#[tokio::test]
async fn vdfs_watch_forwards_session_changes() {
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());
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
// `plugin/tests.rs`；这里只测**经 provider 访问**的行为。

/// 配置文件是 `ext = form` 的可写文档，**但它不在会话清单里**。
///
/// 「清单 = 业务列表」：`.vdfs/session` 下应当只有会话。配置文件进设置菜单走
/// 的是 ConfigurableVisitor 那条通道（`announce_configurable`），不靠清单并列
/// ——否则列表底部会多出一个「设置」项。可达性不受影响：`stat` / `read` 照常。
#[tokio::test]
async fn config_document_is_reachable_but_not_a_session_list_item() {
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());

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
    let p = SessionPlugin::new(None, SessionConfig::default(), test_dir());
    let before = p.config.read().await.max_messages;
    let bad = vdfs::VdfsContent::text("", r#"{"max_messages": 1}"#);
    match p.write(&vctx(), PLUGIN_FILE, &bad).await {
        Err(vdfs::VdfsError::Invalid(v)) => assert_eq!(v.fields[0].field, "max_messages"),
        other => panic!("应为字段级校验错误，实得 {other:?}"),
    }
    assert_eq!(p.config.read().await.max_messages, before);
}
