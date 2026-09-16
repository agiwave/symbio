//! `plugin` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `plugin.rs` 只保留生产代码，测试全部放本文件。

use super::*;
// trait 方法（list/stat/read/write/delete/watch/unwatch）需 trait 在作用域内才可解析
use crate::symbio_core::vdfs_provider::VdfsProvider;
// 目录树场景模块（本文件非测试码用 `super::workdir`；测试模块需显式引入）
use crate::plugins::session::workdir;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::plugins::session::test_dir;

/// 验证 session 存储目录**只**从 HomedirRegistry 派生，不依赖 config；
/// 且它就是宿主层的资源类别根（`category_dir(PLUGIN_SESSION)`）——
/// 会话因此不再手拼一份 `<homedir>/plugins/<类别>` 布局。
#[test]
fn test_session_storage_dir_from_homedir() {
    let dir = SessionPlugin::session_storage_dir();
    let expected = crate::symbio_core::HomedirRegistry::get()
        .join("plugins")
        .join("session");
    assert_eq!(
        dir, expected,
        "session_storage_dir 必须等于 <homedir>/plugins/session"
    );
    assert_eq!(
        dir,
        crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION),
        "会话存储根必须与 VDFS 资源类别根同一条构造式"
    );
    assert!(
        dir.is_absolute(),
        "session_storage_dir 必须是绝对路径: {}",
        dir.display()
    );
}

/// 验证 SessionConfig 不再包含已删除的死字段：
/// - `storage_dir`（存储根由 HomedirRegistry 统一决定）
/// - `session_id`（零消费者；id 由会话目录名决定，配置内自指冗余）
#[test]
fn test_session_config_has_no_dead_fields() {
    let cfg = SessionConfig::default();
    let json = serde_json::to_value(&cfg).unwrap();
    for key in ["storage_dir", "session_id"] {
        assert!(
            json.get(key).is_none(),
            "SessionConfig 不应再包含 {key} 字段, got: {json}"
        );
    }
}

/// 验证从含旧字段的配置反序列化时，未知键被静默忽略（旧 session_config.json
/// 无需迁移即可继续加载）。
#[test]
fn test_session_config_deserialize_ignores_legacy_keys() {
    let json = serde_json::json!({
        "storage_dir": "/tmp/should_be_ignored",
        "session_id": "stale-id-should-be-ignored",
        "max_messages": 42,
    });
    let cfg: SessionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(cfg.max_messages, 42, "max_messages 应被正确反序列化");
}

// ==================== 配置契约一致性（复杂度审计 P0-1 / P1-①）====================

/// `max_tool_rounds` 的契约翻译：0 → `None`（不限制），>0 → `Some(n)`（软上限）。
/// 修复前编排层三处无条件 `Some(...)`，使 chat_loop 的 `None` = 无限语义在主路径不可达。
#[test]
fn max_tool_rounds_zero_maps_to_unlimited() {
    let cfg = SessionConfig {
        max_tool_rounds: 15,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        Some(15),
        "显式 >0 的值应作为软上限下发"
    );

    let cfg = SessionConfig {
        max_tool_rounds: 0,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "0 必须翻译为 None（不限制），而非 Some(0)（0 轮即熔断）"
    );
}

/// 默认配置的产品意图锁定：默认**不设硬性轮次上限**，以显式语义 `0`（不限制）
/// 表达，而非魔法数 65535（旧默认，行为等价但语义含糊）。
#[test]
fn default_max_tool_rounds_is_effectively_unlimited() {
    let cfg = SessionConfig::default();
    assert_eq!(
        cfg.max_tool_rounds, 0,
        "默认轮次上限必须是 0 = 不限制（用户明确要求不设硬上限）"
    );
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "默认配置翻译到 Request 层必须是 None（无限轮次）"
    );
}

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

/// 新建标题：路径名去掉 `.session` 扩展名；空名回落「新对话」
#[test]
fn title_from_new_path_strips_session_ext() {
    assert_eq!(title_from_new_path("我的会话.session"), "我的会话");
    assert_eq!(title_from_new_path("dir/我的会话.session"), "我的会话");
    assert_eq!(title_from_new_path("未命名"), "未命名");
    assert_eq!(title_from_new_path(".session"), "新对话");
    assert_eq!(title_from_new_path(""), "新对话");
}

/// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）、
/// kind / 状态 / 更新时间 / 摘要与 `session_node` 的单点形状同源
#[test]
fn session_node_carries_renderer_ext_and_presentation() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000_000;

    let idle = session_node(&SessionSummary::of(&s), false);
    assert_eq!(idle.name, "abc");
    assert_eq!(idle.effective_ext().as_deref(), Some("session"));
    assert_eq!(idle.kind, PLUGIN_SESSION);
    assert_eq!(idle.status, vdfs::VDFS_STATUS_ACTIVE);
    assert_eq!(idle.updated_at, Some(1_700_000_000_000));
    assert_eq!(idle.access.flags(), "rw", "会话可读可写");
    assert!(!idle.is_dir(), "会话是文档而非目录");

    let busy = session_node(&SessionSummary::of(&s), true);
    assert_eq!(busy.status, vdfs::VDFS_STATUS_WORKING);
}

/// 会话内部寻址（S6/S16）：`<id>` / `<id>/消息[/<mid>]` /
/// `<id>/子会话[/<sub>]` / `<id>/工作目录[/<rel>]`。
/// 未知区段与越界层级一律 NotFound——不给半通不通的路径留口子。
#[test]
fn vdfs_internal_path_parsing() {
    use VdfsSessionPath::*;
    assert!(matches!(parse_session_path("").unwrap(), Root));
    assert!(matches!(parse_session_path("/").unwrap(), Root));
    assert!(matches!(parse_session_path("abc").unwrap(), Session("abc")));
    // 转写列表：目录本身与列表项两级
    assert!(matches!(
        parse_session_path("abc/消息").unwrap(),
        Messages {
            id: "abc",
            mid: None
        }
    ));
    assert!(matches!(
        parse_session_path("abc/消息/m1").unwrap(),
        Messages {
            id: "abc",
            mid: Some("m1")
        }
    ));
    assert!(matches!(
        parse_session_path("abc/子会话").unwrap(),
        SubSessions("abc")
    ));
    assert!(matches!(
        parse_session_path("abc/子会话/s1").unwrap(),
        SubSession {
            id: "abc",
            sub: "s1"
        }
    ));
    assert!(matches!(
        parse_session_path("abc/工作目录").unwrap(),
        Workdir { id: "abc", rel: "" }
    ));
    assert!(matches!(
        parse_session_path("abc/工作目录/src/lib.rs").unwrap(),
        Workdir {
            id: "abc",
            rel: "src/lib.rs"
        }
    ));
    // 未知区段、列表项越界层级 → NotFound
    assert!(parse_session_path("abc/nope").is_err());
    assert!(parse_session_path("abc/消息/m1/deeper").is_err());
    assert!(parse_session_path("abc/子会话/s1/deeper").is_err());
}

/// 会话内部的三个虚拟子目录：转写列表恒在，工作目录按会话是否声明 workdir 出现
#[test]
fn vdfs_internal_dirs_conditional() {
    let without = internal_dirs(false);
    assert_eq!(without.len(), 2);
    assert_eq!(without[0].name, SEG_MESSAGES, "转写列表恒在（会话的本体）");
    assert_eq!(without[1].name, workdir::SEG_SUB_SESSIONS);
    assert!(without.iter().all(|n| n.is_dir()));

    let with = internal_dirs(true);
    assert_eq!(with.len(), 3);
    assert_eq!(with[2].name, workdir::SEG_WORKDIR);
    assert!(
        with.iter().all(|n| n.is_dir() && !n.access.write),
        "三个内部区段都是只读目录（工作目录不提供新建）"
    );
}

/// 转写列表项：`ext = message`、只读、正文进内容、结构进 `attributes`
#[test]
fn vdfs_message_node_splits_text_and_structure() {
    let mut m = cm::ChatMessage {
        id: "m1".into(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        content: Some(cm::MessageContent::Text("你好，世界".into())),
        status: Some(cm::MessageStatus::Completed),
        seq: Some(7),
        timestamp: Some(1_700_000_000_000),
        ..Default::default()
    };
    let n = message_node(&m);
    assert_eq!(n.name, "m1");
    assert_eq!(n.effective_ext().as_deref(), Some("message"));
    assert_eq!(n.title, "助手");
    assert_eq!(n.description.as_deref(), Some("你好，世界"));
    assert_eq!(n.access.flags(), "r", "消息是只读列表项");
    assert_eq!(n.updated_at, Some(1_700_000_000_000));
    assert_eq!(n.status, vdfs::VDFS_STATUS_ACTIVE, "completed 落常规态");
    // 结构字段全在 attributes 里（VDFS 只透传）
    assert_eq!(n.attributes.get("seq"), Some(&json!(7)));
    assert_eq!(n.attributes.get("role"), Some(&json!("assistant")));
    assert_eq!(n.attributes.get("type"), Some(&json!("text")));

    // 正文即内容；流式追加的正是它
    assert_eq!(message_text(&m), "你好，世界");

    // 工具调用：标题补工具名、状态词直通、正文退化为 JSON 视图
    m.msg_type = Some(cm::MessageType::ToolCall);
    m.name = Some("vdfs_list".into());
    m.status = Some(cm::MessageStatus::Streaming);
    assert_eq!(message_node(&m).title, "助手 · vdfs_list");
    assert_eq!(message_node(&m).status, "streaming");
    assert!(message_text(&m).contains("vdfs_list"));

    // 失败态：错误原因也在 attributes 里（前端按 ext 自行取用）
    m.status = Some(cm::MessageStatus::Failed);
    m.error = Some("模型超时".into());
    assert_eq!(message_node(&m).status, "failed");
    assert_eq!(
        message_node(&m).attributes.get("error"),
        Some(&json!("模型超时"))
    );
}

fn msg(id: &str, seq: Option<i64>) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.into(),
        seq,
        ..Default::default()
    }
}

/// 带父指针的消息（一个 Turn = 根 + 它的全部后代）
fn child(id: &str, seq: Option<i64>, parent: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.into(),
        seq,
        parent_id: Some(parent.into()),
        ..Default::default()
    }
}

fn ids(v: &[cm::ChatMessage]) -> Vec<&str> {
    v.iter().map(|m| m.id.as_str()).collect()
}

/// 没给窗口参数 = 全量：流式期间前端要的就是完整列表，行为必须与从前一致
#[test]
fn transcript_window_without_params_is_the_whole_list() {
    let msgs = vec![msg("t1", Some(1)), child("r1", Some(2), "t1")];
    assert_eq!(ids(&transcript_window(&msgs, None, None)), vec!["t1", "r1"]);
}

/// 父节点闭合：只要根被选中，它的**全部**后代都在窗口里；窗口里也不会出现
/// 父节点在窗口外的孤儿（那样前端拼不成树）
#[test]
fn transcript_window_is_parent_closed() {
    let msgs = vec![
        msg("t1", Some(1)),
        child("r1", Some(2), "t1"),
        msg("t2", Some(3)),
        child("r2", Some(4), "t2"),
    ];
    // limit=1 → 只取最后一个根（t2），但带上它的后代 r2
    assert_eq!(
        ids(&transcript_window(&msgs, Some(1), None)),
        vec!["t2", "r2"]
    );
    // limit=2 → 两个根 + 两个后代
    assert_eq!(
        ids(&transcript_window(&msgs, Some(2), None)),
        vec!["t1", "r1", "t2", "r2"]
    );
    // parent 指向不存在 / 空 ⇒ 自己即根（孤儿不被丢弃）
    let orphan = vec![msg("t1", Some(1)), child("o1", Some(2), "nope")];
    assert_eq!(ids(&transcript_window(&orphan, Some(1), None)), vec!["o1"]);
}

/// 游标往前翻页：从游标所在根**之前**再取 `limit` 个根
#[test]
fn transcript_window_pages_before_cursor() {
    let msgs = vec![msg("t1", Some(1)), msg("t2", Some(2)), msg("t3", Some(3))];
    assert_eq!(
        ids(&transcript_window(&msgs, Some(2), Some("t3"))),
        vec!["t1", "t2"]
    );
    assert_eq!(
        ids(&transcript_window(&msgs, Some(1), Some("t2"))),
        vec!["t1"]
    );
    // 游标可以写成地址形式（`<…>/<id>`），不只是裸 id
    assert_eq!(
        ids(&transcript_window(
            &msgs,
            Some(2),
            Some(".vdfs/session/s/消息/t3")
        )),
        vec!["t1", "t2"]
    );
    // 游标在第一页之前 ⇒ 空页（自然收敛，不报错）
    assert!(transcript_window(&msgs, Some(2), Some("t1")).is_empty());
}

/// **Turn 永不被劈开**：`limit` 是名义值，实际条数恒 ≥ limit
///
/// 这是「计量单位是根、不是条」的锁死测试——按条数截断会把一轮 reason /
/// tool_call 与它的根分开，前端树就拼不起来。
#[test]
fn transcript_window_keeps_turns_whole() {
    let msgs = vec![
        msg("t2", Some(1)),
        child("r2", Some(2), "t2"),
        child("c2", Some(3), "t2"),
        msg("t3", Some(4)),
    ];
    // 无游标 ⇒ 取最新的根：t3（只有它自己）
    assert_eq!(ids(&transcript_window(&msgs, Some(1), None)), vec!["t3"]);
    // 游标 t3 ⇒ 前一个根 t2，**连同它的两个后代**一起回来
    assert_eq!(
        ids(&transcript_window(&msgs, Some(1), Some("t3"))),
        vec!["t2", "r2", "c2"],
        "limit=1 却返回 3 条 —— 一个 Turn 不能拆"
    );
}

/// 列表顺序以 `seq` 为准（唯一权威顺序锚点）：缺 `seq` 的排最后且不打乱相对顺序
#[test]
fn vdfs_messages_are_ordered_by_seq() {
    let msgs = vec![
        msg("c", Some(30)),
        msg("x", None),
        msg("a", Some(10)),
        msg("y", None),
        msg("b", Some(20)),
    ];
    let ids: Vec<String> = ordered(msgs).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["a", "b", "c", "x", "y"]);
}

/// 在途消息叠加在落库转写之上：同 id 时**在途胜出，但继承落库的 `seq`**。
///
/// 继承 `seq` 是关键：不继承的话同一条消息会以「有 seq / 无 seq」两种形态
/// 被排到列表的两个位置（一份在历史里、一份在末尾）。
#[test]
fn overlay_live_keeps_seq_from_stored() {
    let mut stored = msg("m1", Some(5));
    stored.content = Some(cm::MessageContent::Text("落库正文".into()));
    stored.status = Some(cm::MessageStatus::Completed);
    let mut live = msg("m1", None);
    live.content = Some(cm::MessageContent::Text("落库正文 + 流式追加".into()));
    live.status = Some(cm::MessageStatus::Streaming);

    let merged = overlay_live(vec![stored], vec![live, msg("m2", None)]);
    let ids: Vec<String> = ordered(merged).into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["m1", "m2"], "m1 继承 seq=5 仍在 m2（无 seq）之前");

    // 在途版本的内容与状态胜出
    let merged = overlay_live(
        vec![msg("m1", Some(5))],
        vec![{
            let mut l = msg("m1", None);
            l.content = Some(cm::MessageContent::Text("增量".into()));
            l
        }],
    );
    assert_eq!(merged.len(), 1, "同 id 不得出现两条");
    assert_eq!(merged[0].seq, Some(5), "顺序锚点由落库版本继承");
    assert_eq!(message_text(&merged[0]), "增量");
}

/// 补丁 → 变更的映射：新 id 是 `created`，有增量是 `appended`，其余 `updated`。
///
/// 同时钉住**载荷宽度**：`created` / `updated` 带节点视图与内容快照（冷路径免回读），
/// `appended` **只**带增量（热路径逐帧，多一个字段就是 O(n²)）。
#[test]
fn message_change_maps_patch_to_change_kind() {
    let mut patch = msg("m1", None);
    patch.content = Some(cm::MessageContent::Text("正文".into()));

    // 新 id
    let c = message_change("abc", &patch, false, None);
    assert_eq!(c.change, vdfs::VDFS_CHANGE_CREATED);
    assert_eq!(c.path, "abc/消息/m1", "变更地址与 list 的节点地址同源");
    assert!(c.delta.is_none());
    // 载荷：结构（attributes）与正文各自就位，消费者无需回读
    assert_eq!(
        c.node.as_ref().map(|n| n.path.as_str()),
        Some("abc/消息/m1"),
        "载荷节点的路径与事件路径同口径（分发层据此补前缀）"
    );
    assert_eq!(
        c.node.as_ref().and_then(|n| n.effective_ext()),
        Some(vdfs::VDFS_EXT_MESSAGE.to_string())
    );
    assert_eq!(c.content.as_deref(), Some("正文"));

    // 已有 id + 增量 → appended（只带 delta，不带节点视图 / 内容快照）
    let c = message_change("abc", &patch, true, Some("追加".into()));
    assert_eq!(c.change, vdfs::VDFS_CHANGE_APPENDED);
    assert_eq!(c.delta.as_deref(), Some("追加"));
    assert!(
        c.node.is_none() && c.content.is_none(),
        "追加是热路径：载荷必须保持只有增量"
    );

    // 已有 id、无增量（状态迁移 / 全量替换）→ updated（同样带全量载荷）
    let c = message_change("abc", &patch, true, None);
    assert_eq!(c.change, vdfs::VDFS_CHANGE_UPDATED);
    assert!(c.delta.is_none());
    assert!(c.node.is_some() && c.content.is_some());

    // 空增量不算追加（避免发一条什么都不带的 appended 让消费者空转）
    let c = message_change("abc", &patch, true, Some(String::new()));
    assert_eq!(c.change, vdfs::VDFS_CHANGE_UPDATED);
}

/// 地址的「拼」与「解」互逆——改地址方案时漏改一边会被这条挡住
#[test]
fn message_path_round_trips_through_parser() {
    let p = message_path("abc", "m1");
    assert_eq!(p, "abc/消息/m1");
    assert!(matches!(
        parse_session_path(&p).unwrap(),
        VdfsSessionPath::Messages {
            id: "abc",
            mid: Some("m1")
        }
    ));
}

// 工作目录节点的形状由 `workdir::tree_node` 单点保证（见其单测）：
// 目录 `l`、文件 `rw`——本文件不再另设一份转换逻辑，因此无需重复断言。

/// 会话节点自带清单字段（S8）：`message_count` / `metadata` / `meta_tags`
/// 挂在 flatten 的 attributes 上，使会话清单无需再走一次额外的列接口
#[test]
fn vdfs_session_node_carries_list_fields() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000;
    s.metadata = json!({ "workdir": "/tmp/proj/demo", "title": "T" });

    let n = session_node(&SessionSummary::of(&s), false);
    assert_eq!(n.attributes.get("message_count"), Some(&json!(0)));
    assert_eq!(
        n.attributes.get("metadata").and_then(|v| v.get("workdir")),
        Some(&json!("/tmp/proj/demo"))
    );
    let tags = n
        .attributes
        .get("meta_tags")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(tags.len(), 2, "工作目录名 + 消息数");
    assert_eq!(tags[0], json!("demo"), "标签取工作目录 basename");
    assert_eq!(tags[1], json!("0 条"));

    // is_working 仍由 status 承载（机制口径，不另设 is_working 字段）
    assert_eq!(n.status, vdfs::VDFS_STATUS_ACTIVE);
    assert_eq!(
        session_node(&SessionSummary::of(&s), true).status,
        vdfs::VDFS_STATUS_WORKING
    );
}

/// 会话内容（VDFS `read`）：转写全文 + 元数据，JSON
#[test]
fn vdfs_session_content_is_json() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000_000;
    let c = session_content(&s).unwrap();
    let text = c.text.as_deref().unwrap_or("");
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["id"], "abc");
    assert!(v.get("messages").is_some(), "聊天转写随内容下发");
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

// ==================== 配置文档（`.vdfs/session/PLUGIN.yml`） ====================

/// **定义与配置同源**：面板字段的默认值一律来自 `SessionConfig::default()`，
/// 且 serde 默认值函数与 `Default` impl 不漂移（两处各自书写必然漂移）。
#[test]
fn config_definition_defaults_come_from_session_config() {
    let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
    let def = config_definition();
    let fields = &def.sections[0].fields;
    assert!(!fields.is_empty(), "会话配置必须有字段");
    for f in fields {
        let declared = f
            .default
            .clone()
            .unwrap_or_else(|| panic!("字段 {} 缺少 default", f.key));
        let actual = defaults
            .get(&f.key)
            .unwrap_or_else(|| panic!("SessionConfig 不存在字段 {}", f.key));
        assert_eq!(
            &declared, actual,
            "面板 {}.default={declared} 与 SessionConfig::default().{}={actual} 不一致",
            f.key, f.key
        );
    }

    let from_empty: SessionConfig =
        serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
    assert_eq!(
        serde_json::to_value(&from_empty).unwrap(),
        defaults,
        "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
    );
}

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
