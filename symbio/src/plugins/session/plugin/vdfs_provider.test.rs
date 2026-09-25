//! `plugin/vdfs_provider.rs` 的单元测试（`impl VdfsProvider` 的对外行为）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
// dispatch 是 VdfsProvider 的唯一入口（测试经文件尾的 LIST/STAT/… 请求常量调用）
use crate::symbio_core::vdfs_provider::VdfsProvider;
// 地址构造辅助：只被本测试用，故不经 `plugin.rs` 的共享面转出（那里会让
// `unused_imports` 误报——它看不见「仅经 glob 链使用」的再导出）。
use crate::plugins::session::plugin::nodes::{message_dir_path, message_path};
use crate::symbio_core::vdfs_provider::VdfsChange;
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
    let meta = p.meta();
    assert_eq!(meta.name, "会话");
    assert_eq!(meta.icon.as_deref(), Some("session"));
    // 顺序由本插件自述自持（**单一真相源**），
    // 使 `<根>` 左栏与详情恒等
    assert_eq!(meta.order, 1);
    assert_eq!(meta.root_access.flags(), "l");
    assert!(!meta.root_access.traverse, "会话是叶子，不参与树遍历");

    // 根下可新建「会话」——类型即「新建」入口的唯一依据。
    // 它不在同步的 `PluginMeta` 上（schema 需运行期汇流），由 provider 在
    // **根节点自己的自述**里现场给（`Stat("")`——与更深层节点同一条通道，
    // 见 `VdfsNode::new_type`）。
    let root = p
        .dispatch(&vctx(), "", STAT)
        .await
        .unwrap()
        .into_stat()
        .expect("根节点自述");
    let t = root.new_type.expect("根下可新建会话");
    assert_eq!(t.ext, EXT_SESSION);
    assert_eq!(t.title, "会话");
    // 草稿节点与落成后走**同一个渲染器**（`ext = session`，不是通用表单），
    // 故不声明 `node_ext`：新建会话直接进会话详情页
    assert!(t.node_ext.is_none());

    // 新建会话是**草稿态**：会话还不存在，没有节点可挂 `schema`，故定义挂在**类型**上
    // ——选项行在会话创建前就要完整渲染（草稿选择随 `create` 一次写入 metadata）。
    // 本 fixture 未装配容器 ⇒ 收集不到任何贡献方 ⇒ 字段表为空；但定义本身**不缺席**
    // （缺席会让前端把「草稿态」误当成「没有选项」）。
    let schema = t.schema.as_ref().expect("新建类型必须带选项定义");
    assert_eq!(schema["binding"], serde_json::json!("option"));
    assert_eq!(schema["sections"][0]["fields"], serde_json::json!([]));
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
    assert!(p.dispatch(&vctx(), &id, LIST).await.is_err());
    assert!(p.dispatch(&vctx(), &id, STAT).await.is_err());
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

    let n = p
        .dispatch(&vctx(), &id, STAT)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
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

    p.dispatch(&vctx(), "", vdfs::VdfsRequest::Watch { sink: sink.clone() })
        .await
        .unwrap();
    p.dispatch(&vctx(), "", vdfs::VdfsRequest::Watch { sink })
        .await
        .unwrap();
    assert_eq!(p.change_subs.subscriber_count(), 2);
    assert_eq!(p.change_subs.paths(), vec!["".to_string()]);

    p.notify_change("abc");

    // 投递是同步的，但两条订阅共用一个投递器 ⇒ 恰好一次
    let got = rx.recv().await.expect("变更应经 watch 投递到 sink");
    assert_eq!(got.path, "abc");
    assert!(got.data.is_none(), "资源信号无载荷");
    assert!(rx.try_recv().is_err(), "同一路径的重复订阅不得收到重复帧");

    p.dispatch(&vctx(), "", UNWATCH).await.unwrap();
    assert_eq!(
        p.change_subs.subscriber_count(),
        1,
        "取消一位仍有另一位，不得提前停投"
    );
    p.dispatch(&vctx(), "", UNWATCH).await.unwrap();
    assert!(
        !p.change_subs.has_subscribers(),
        "unwatch 必须把该路径彻底摘掉"
    );
    p.notify_change("abc");
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
    let node = p
        .dispatch(&vctx(), PLUGIN_FILE, STAT)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(node.name, PLUGIN_FILE, "地址就是插件目录里的真实文件名");
    assert_eq!(node.ext.as_deref(), Some(vdfs::VDFS_EXT_FORM));
    assert_eq!(node.access.flags(), "rw");
    assert!(node.schema.is_some(), "定义随节点下发");

    // 不并列：清单里没有它
    let items = p
        .dispatch(&vctx(), "", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert!(
        !items.iter().any(|it| it.node.name == PLUGIN_FILE),
        "会话清单里只应有会话，配置文件不该出现"
    );

    // 配置文件不是会话 id：按文件读，不按会话解析
    assert_eq!(
        p.dispatch(&vctx(), PLUGIN_FILE, STAT)
            .await
            .unwrap()
            .into_stat()
            .unwrap()
            .name,
        PLUGIN_FILE
    );
    let content = p
        .dispatch(&vctx(), PLUGIN_FILE, READ)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    let cfg: SessionConfig = serde_json::from_str(content.text.as_deref().unwrap()).unwrap();
    assert_eq!(cfg.max_messages, SessionConfig::default().max_messages);

    // 文档没有子项，也不可删除
    assert!(p.dispatch(&vctx(), PLUGIN_FILE, LIST).await.is_err());
    assert!(p.dispatch(&vctx(), PLUGIN_FILE, DEL).await.is_err());
}

/// 会话清单里的每一项都带**选项定义**（`schema`）——前端零额外请求即可渲染选项栏
/// （旧形态要一次 `options/list`）。
///
/// 定义与「是哪个会话」无关（值走 `attributes.metadata`），故清单里逐项挂的是
/// **同一份**；设计文档 §7 已量化这份重复（~2 KB/项）并接受它，换来的正是
/// 「清单一次取全」。
#[tokio::test]
async fn session_list_carries_the_option_definition() {
    let (_dir, p) = fixture();
    let created = p
        .dispatch(
            &vctx(),
            "",
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text("").with_create(),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(created.created);
    // 匿名写（打在目录自身）→ provider 交回**自己生成的名字**；地址由调用方拼
    let id = created.name.expect("匿名写必须交回新条目的名字");

    let items = p
        .dispatch(&vctx(), "", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let node = items
        .iter()
        .find(|it| it.node.name == id)
        .expect("清单里有这个会话");

    let schema = node.node.schema.as_ref().expect("会话节点必须带选项定义");
    assert_eq!(schema["binding"], serde_json::json!("option"));
    assert_eq!(
        schema["title_fallback"],
        serde_json::json!("会话选项"),
        "定义要能自述——它是 `node.schema` 上唯一的呈现契约"
    );
    // 本 fixture 未装配容器 ⇒ 收集不到贡献方 ⇒ 字段表为空；生产路径由容器广播出
    // 工作目录 / 智能体 / Model / 风险 / 模式 / 心跳 六项
    assert_eq!(schema["sections"][0]["fields"], serde_json::json!([]));

    // 当前值走 `attributes.metadata`（新形态的值载体）：写一次 metadata，
    // 清单节点上立刻能看到——「定义 + 值 + 落库」三条通路因此都在 VDFS 上闭环，
    // 不需要第二套协议（旧形态要 `options/list` + `session/update` 各一条，两条都已退役）
    p.dispatch(
        &vctx(),
        &id,
        vdfs::VdfsRequest::Write {
            content: vdfs::VdfsContent::text(r#"{"metadata":{"risk_level":"high"}}"#),
        },
    )
    .await
    .unwrap()
    .into_write()
    .unwrap();
    let items = p
        .dispatch(&vctx(), "", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let node = items
        .iter()
        .find(|it| it.node.name == id)
        .expect("会话还在");
    assert_eq!(
        node.node.attributes["metadata"]["risk_level"],
        serde_json::json!("high"),
        "写入的 metadata 必须原样出现在节点 attributes 上"
    );
}

// ==================== 新建语义：id 从哪来 ====================
//
// 「**有名字**时 id 来自地址（使用方给），**没名字**时 id 由 provider 生成」是
// VDFS 的通用规则（`vdfs_service::entry::id_of` 的注释、`VdfsProvider::write`
// 的「两种目标形态」表）。会话曾经是唯一例外——无论有没有名字都自己生成 id、
// 把名字只当标题。2026-09-23 对齐，下面三例把三条契约钉住。

/// 具名新建：**就地创建**，地址末段即会话 id。
///
/// 这是 CLI 的「客户端指定会话 id」得以走 `vdfs/write` 的全部依据
/// （旧 `session/update` 路由唯一的独有能力）。
#[tokio::test]
async fn named_create_uses_the_address_as_the_session_id() {
    let (_dir, p) = fixture();
    let r = p
        .dispatch(
            &vctx(),
            "cli-abc",
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text(r#"{"metadata":{"workdir":"/w"}}"#).with_create(),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(r.created, "具名目标不存在 ⇒ 就地创建");
    assert!(
        r.name.is_none(),
        "具名写不回传地址：地址就是使用方写的那一个"
    );

    let s = p.session_of("cli-abc").await.unwrap();
    assert_eq!(s.id, "cli-abc");
    assert_eq!(s.metadata["workdir"], serde_json::json!("/w"));
    assert!(
        s.metadata.get("title").is_none(),
        "名字是**身份**不是标题：把 id 抄成标题会让 `--session cli18f3a2` 污染侧栏"
    );
    assert_eq!(s.display_title(), "新对话", "无标题 ⇒ 仍由首条消息派生");
}

/// 具名目标**已存在** + `create` ⇒ 覆盖（浅合并），**不重复创建**。
///
/// 锁的是 `create` 位的定义：「只回答**不存在时**怎么办」。CLI 的 `ensure_session`
/// 因此是**一次**调用同时覆盖「新建」与「改元数据」——旧路由的 upsert 整条落在这里。
#[tokio::test]
async fn named_create_on_an_existing_session_merges_instead_of_duplicating() {
    let (_dir, p) = fixture();
    let first = p
        .dispatch(
            &vctx(),
            "keep",
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text(
                    r#"{"metadata":{"workdir":"/old","agent_id":"a"}}"#,
                )
                .with_create(),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(first.created);

    let again = p
        .dispatch(
            &vctx(),
            "keep",
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text(r#"{"metadata":{"workdir":"/new"}}"#)
                    .with_create(),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(!again.created, "已存在 ⇒ 覆盖，不是再建一个");
    assert!(again.name.is_none(), "具名写不回传地址");

    let s = p.session_of("keep").await.unwrap();
    assert_eq!(
        s.metadata["workdir"],
        serde_json::json!("/new"),
        "提供的键被覆盖"
    );
    assert_eq!(
        s.metadata["agent_id"],
        serde_json::json!("a"),
        "未提供的键保持不变（浅合并）"
    );
    assert_eq!(
        p.dispatch(&vctx(), "", LIST)
            .await
            .unwrap()
            .into_list()
            .unwrap()
            .len(),
        1,
        "不该造出第二个会话"
    );
}

/// 覆盖分支：metadata 浅合并 + 显式 `title` 写进 `metadata.title`。
///
/// 原 `handlers.test.rs::session_update_and_vdfs_write_agree_on_metadata` 的契约
/// （两条写入路径产出逐字相同的 metadata）随 `session/update` 退役失去意义——
/// 只剩一条路径，分歧在结构上写不出来。契约本身搬来这里继续守着。
#[tokio::test]
async fn write_merges_metadata_shallowly() {
    let (_dir, p) = fixture();
    p.dispatch(
        &vctx(),
        "s1",
        vdfs::VdfsRequest::Write {
            content: vdfs::VdfsContent::text(
                r#"{"metadata":{"workdir":"/old","agent_id":"keep-me"}}"#,
            )
            .with_create(),
        },
    )
    .await
    .unwrap()
    .into_write()
    .unwrap();
    p.dispatch(
        &vctx(),
        "s1",
        vdfs::VdfsRequest::Write {
            content: vdfs::VdfsContent::text(r#"{"metadata":{"workdir":"/new"},"title":"改名"}"#),
        },
    )
    .await
    .unwrap()
    .into_write()
    .unwrap();

    let s = p.session_of("s1").await.unwrap();
    assert_eq!(s.metadata["workdir"], serde_json::json!("/new"));
    assert_eq!(
        s.metadata["agent_id"],
        serde_json::json!("keep-me"),
        "未提到的键保持不变"
    );
    assert_eq!(s.metadata["title"], serde_json::json!("改名"));
    assert_eq!(s.display_title(), "改名");
}

/// 配置写入：校验先于一切（字段级错误），坏值不会改动内存
#[tokio::test]
async fn config_write_validates_before_applying() {
    let (_dir, p) = fixture();
    let before = p.config.read().await.max_messages;
    let bad = vdfs::VdfsContent::text(r#"{"max_messages": 1}"#);
    match p
        .dispatch(
            &vctx(),
            PLUGIN_FILE,
            vdfs::VdfsRequest::Write { content: bad },
        )
        .await
    {
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
    let items = p
        .dispatch(&vctx(), &id, LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let mem = items
        .iter()
        .find(|it| it.node.name == crate::symbio_core::AGENTS_FILE)
        .expect("会话内部应列出记忆文件");
    assert!(!mem.node.is_dir(), "记忆是文件，不是目录");
    assert_eq!(
        mem.node.access.flags(),
        "rw",
        "模型与用户共用这一份，可读可写"
    );
    assert_eq!(mem.node.kind, PLUGIN_SESSION);
    assert!(mem.node.description.is_some(), "列表里要能看出它是干什么的");

    // ② `stat` 与 `list` 同源（同一份形状，不是另写一份「详情版」）
    let stat = p
        .dispatch(&vctx(), &path, STAT)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(stat.name, mem.node.name);
    assert_eq!(stat.access.flags(), mem.node.access.flags());
    assert_eq!(stat.size, mem.node.size);
    assert_eq!(stat.description, mem.node.description);

    // ③ 还没写过 → 空串（「还没写过」是记忆的正常状态，不是错误）
    assert_eq!(
        p.dispatch(&vctx(), &path, READ)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .text
            .as_deref(),
        Some("")
    );

    // ④ 写入 → 读回（写闸门在内核，本层不重复实现）
    let r = p
        .dispatch(
            &vctx(),
            &path,
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text("本会话约定：所有时间用 UTC。"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(r.created, "首次写入应报 created");
    assert_eq!(
        p.dispatch(&vctx(), &path, READ)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .text
            .as_deref(),
        Some("本会话约定：所有时间用 UTC。")
    );
    let again = p
        .dispatch(
            &vctx(),
            &path,
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text("改主意了"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(!again.created, "覆盖写入不是 created");

    // ⑤ 记忆不可删除：要清空就写空内容（一次可读、可审、可撤销的显式动作）
    assert!(
        matches!(
            p.dispatch(&vctx(), &path, DEL).await,
            Err(vdfs::VdfsError::Forbidden(_))
        ),
        "删除即丢失本会话的长期约定，必须明确拒绝"
    );
    assert!(
        p.dispatch(&vctx(), &path, READ).await.is_ok(),
        "拒绝删除后内容仍在"
    );

    // ⑥ 记忆是文件：没有子项；更深层级也不解析
    assert!(p.dispatch(&vctx(), &path, LIST).await.is_err());
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
        p.dispatch(
            &vctx(),
            &path,
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text("12345")
            }
        )
        .await
        .is_err(),
        "超出写入上限必须被拒绝"
    );
    assert_eq!(
        p.dispatch(&vctx(), &path, READ)
            .await
            .unwrap()
            .into_read()
            .unwrap()
            .text
            .as_deref(),
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

    assert!(p.dispatch(&vctx(), &path, STAT).await.is_err());
    assert!(p.dispatch(&vctx(), &path, READ).await.is_err());
    assert!(p
        .dispatch(
            &vctx(),
            &path,
            vdfs::VdfsRequest::Write {
                content: vdfs::VdfsContent::text("x")
            }
        )
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
    p.dispatch(&vctx(), &id, vdfs::VdfsRequest::Watch { sink })
        .await
        .unwrap();

    p.dispatch(
        &vctx(),
        &path,
        vdfs::VdfsRequest::Write {
            content: vdfs::VdfsContent::text("记一笔"),
        },
    )
    .await
    .unwrap()
    .into_write()
    .unwrap();

    let got = rx.recv().await.expect("写入应投递一条变更");
    assert_eq!(
        got.path,
        format!("{id}/{}", crate::symbio_core::AGENTS_FILE),
        "变更路径与 list 返回的节点地址同源"
    );
    assert!(got.data.is_none(), "资源信号无载荷（信封没有操作枚举）");
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
        ..vdfs::VdfsContent::text("{}")
    };
    let r = p
        .dispatch(&vctx(), "", vdfs::VdfsRequest::Write { content })
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(r.created);

    let id = r.name.expect("匿名写必须交回新条目的名字");
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
            .dispatch(
                &vctx(),
                "",
                vdfs::VdfsRequest::Write {
                    content: vdfs::VdfsContent {
                        create: true,
                        ..vdfs::VdfsContent::text("{}")
                    },
                },
            )
            .await
            .unwrap()
            .into_write()
            .unwrap();
        let id = r.name.expect("匿名写必须交回新条目的名字");
        assert!(ids.insert(id.clone()), "id 重复：{id}");
    }
    assert_eq!(ids.len(), 32);
}

// ==================== 转写区段（`<根>/session/<id>/message`）的三个入口 ====================
//
// 消息的**改写 / 截断 / 清空**曾经各有专用路由（`chat/update_message` /
// `chat/delete_message` / `chat/clear_messages`），2026-09-18 迁到 VDFS：
//
// | 操作 | 入口 | 落到 VDFS 变更上的形状 |
// |---|---|---|
// | 改写某条 | `write(<id>/message/<mid>)` | 该消息一条变更（**不带**载荷 ⇒ 消费端回读） |
// | 删该条及其后 | `action(<id>/message/<mid>, "truncate")` | 被删的各一条移除帧 + 回执带被删 id |
// | 清空历史 | `action(<id>/message, "clear")` | 每条一条移除帧 |
//
// 消息域的实时面与历史面合流为同一条 `vdfs/watch`（ADR-025）。断言面是
// 「投递了几条变更、每条说了什么」（`watch_changes`）：截断与清空三例各自钉住
// 这正是当前机制要守的边界（删除**逐条**下发，消费端不必整份重读；
// 什么都没删**一条都不发**）。
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
    p.dispatch(&vctx(), &message_dir_path(id), LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap()
        .into_iter()
        .map(|it| it.node.name)
        .collect()
}

/// 在本插件的变更表上挂一个收件盒（收 `VdfsChange`）。
///
/// 收的是 **provider 子树内的相对路径**（`<id>/message/<mid>`）——门面补挂载前缀是
/// 分发层的事，不在这里发生（见 `VdfsChange` 的文档）。
///
/// 与「订阅进程级全局总线再过滤」相比有两点，都是**变简单**：
/// - 订阅表是**本插件实例**的（`SessionPlugin::change_subs`），不是进程级全局表，
///   因此不必按 `session_id` 过滤——并行用例互不干扰；
/// - 收的是 `VdfsChange`（`path` + 可选 `data`），不必拆信封，也不必
///   为「通道满 → resync」准备容量。
fn watch_changes(p: &SessionPlugin) -> std::sync::Arc<std::sync::Mutex<Vec<VdfsChange>>> {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    p.change_subs.watch(
        "",
        std::sync::Arc::new(move |c: VdfsChange| sink.lock().unwrap().push(c)),
    );
    seen
}

/// 截断某条：**它及其之后**全部删除，回执带权威的被删 id 列表。
#[tokio::test]
async fn truncate_removes_the_target_and_everything_after() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-truncate");
    seed_messages(&p, &id, &["一", "二", "三", "四"]).await;
    let seen = watch_changes(&p);

    let r = p
        .dispatch(
            &vctx(),
            &message_path(&id, "m1"),
            vdfs::VdfsRequest::Action {
                action: vdfs::VDFS_ACTION_TRUNCATE.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap()
        .into_action()
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

    let changes = seen.lock().unwrap().clone();
    assert_eq!(changes.len(), 3, "被删的三条各发一条删除帧");
    for (c, expect) in changes.iter().zip(["m1", "m2", "m3"]) {
        // 落点是那条消息**节点自身**的地址（`<sid>/message/<mid>`），身份即末段；
        // 删除语义在 data.status = removed
        assert_eq!(
            c.path,
            message_path(&id, expect),
            "落点恒为被变更节点自身的地址"
        );
        let v = c.data.as_ref().expect("删除帧带 removed 状态载荷");
        assert_eq!(v["id"], expect, "删除**逐条**下发");
        assert_eq!(v["status"], "removed");
    }
}

/// 截断一个**不存在**的目标：回执是空列表，列表一条不动，且**一条帧都不发**。
///
/// 「什么都没删」是**结果**不是错误，也不该在转写上留下痕迹——发一条「从 <mid> 起
/// 截断」的通知会让消费端从一条并不存在的节点起截断，把整个列表清空。
#[tokio::test]
async fn truncate_of_missing_target_changes_nothing() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-truncate-miss");
    seed_messages(&p, &id, &["一", "二"]).await;
    let seen = watch_changes(&p);

    let r = p
        .dispatch(
            &vctx(),
            &message_path(&id, "nope"),
            vdfs::VdfsRequest::Action {
                action: vdfs::VDFS_ACTION_TRUNCATE.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap()
        .into_action()
        .unwrap();
    assert!(r.ok, "目标不存在是**结果**，不是错误");
    assert_eq!(r.data, Some(json!([])), "回执是空列表");
    assert_eq!(transcript_ids(&p, &id).await, vec!["m0", "m1"], "列表未动");
    assert!(
        seen.lock().unwrap().is_empty(),
        "什么都没删 ⇒ 一条变更都不发"
    );
}

/// 清空：消息全没了，**会话本体保留**（id / metadata / 工作目录）。
#[tokio::test]
async fn clear_empties_the_transcript_but_keeps_the_session() {
    let (_dir, p) = fixture();
    let id = unique_id("msg-clear");
    seed_messages(&p, &id, &["一", "二", "三"]).await;
    let seen = watch_changes(&p);

    let r = p
        .dispatch(
            &vctx(),
            &message_dir_path(&id),
            vdfs::VdfsRequest::Action {
                action: vdfs::VDFS_ACTION_CLEAR.to_string(),
                payload: None,
            },
        )
        .await
        .unwrap()
        .into_action()
        .unwrap();
    assert!(r.ok);
    assert!(transcript_ids(&p, &id).await.is_empty(), "列表应清空");

    let changes = seen.lock().unwrap().clone();
    assert_eq!(changes.len(), 3, "清空＝逐条删除帧");
    assert!(
        changes
            .iter()
            .all(|c| c.data.as_ref().is_some_and(|v| v["status"] == "removed")),
        "每一条都是 removed 状态载荷（信封无 deleted）"
    );

    // 会话本体还在（清空不是删除会话）
    let n = p
        .dispatch(&vctx(), &id, STAT)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
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
                p.dispatch(
                    &vctx(),
                    &path,
                    vdfs::VdfsRequest::Action {
                        action: action.to_string(),
                        payload: None
                    }
                )
                .await,
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

// ==================== dispatch 请求形态（测试辅助） ====================

const LIST: vdfs::VdfsRequest = vdfs::VdfsRequest::List {
    limit: None,
    before: None,
};
const STAT: vdfs::VdfsRequest = vdfs::VdfsRequest::Stat;
const READ: vdfs::VdfsRequest = vdfs::VdfsRequest::Read;
const DEL: vdfs::VdfsRequest = vdfs::VdfsRequest::Delete { recursive: false };
const UNWATCH: vdfs::VdfsRequest = vdfs::VdfsRequest::Unwatch;
