//! `plugin/nodes.rs` 的单元测试（VDFS 路径模型 / 节点构造 / 消息投影）。
//!
//! 与实现同目录分文件（约定同 `store/tests.rs`）：测试跟着被测试的实现走。

use super::*;
// 目录树场景模块的区段常量（`SEG_SUB_SESSIONS` / `SEG_WORKDIR`）
use crate::plugins::session::workdir;

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
