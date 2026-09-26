//! `plugin/nodes.rs` 的单元测试（VDFS 路径模型 / 节点构造 / 消息投影）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
// 目录树场景模块的区段常量（`SEG_SUB_SESSIONS` / `SEG_WORKDIR`）
use crate::plugins::session::workdir;

/// 消息节点序列化后 `node.name` 必须仍是**节点 id**。
///
/// 钉住一次真实事故：attributes 的工具名原先用 `name` 作键，与 `VdfsNode`
/// 结构体自身的 `name` 字段（= 节点 id）在 `#[serde(flatten)]` 序列化下同名冲突
/// ——序列化产物出现两个 `"name"` 键，后者（attributes 的 `null`）覆盖前者，
/// 前端按 `node.name` 寻址的整条实时链路瘫痪（created / updated 全被丢弃，
/// 只剩 appended 增量撑起无类型的幽灵节点）。工具名改用 `tool_name` 后，
/// 这条测试保证「id 不被覆盖 + 工具名仍在」两个事实同时成立。
#[test]
fn message_node_serialized_name_is_node_id_not_tool_name() {
    let m = cm::ChatMessage {
        id: "nodeid123".into(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        status: Some(cm::MessageStatus::Streaming),
        content: Some(cm::MessageContent::Text("你好".into())),
        parent_id: Some("turn1".into()),
        // 文本节点没有工具名 → `m.name = None`（正是事故现场的节点形状）
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(message_node(&m)).unwrap();

    // JSON 层（前端 / 任何 JSON 消费者的视角）：attributes 的 null 不得覆盖节点 id
    assert_eq!(
        v["name"],
        serde_json::json!("nodeid123"),
        "attributes 键与结构体字段同名会在 flatten 序列化时覆盖节点 id"
    );
    assert_eq!(v["parent_id"], serde_json::json!("turn1"));

    // 工具名单独走 `tool_name` 键：None 序列化为 null，不碰 `name`
    assert_eq!(v["tool_name"], serde_json::Value::Null);

    // 带 工具名 的节点（ToolCall）：tool_name 就位，id 仍是节点 id
    let tc = cm::ChatMessage {
        id: "tc456".into(),
        msg_type: Some(cm::MessageType::ToolCall),
        name: Some("local/ls".into()),
        ..Default::default()
    };
    let v = serde_json::to_value(message_node(&tc)).unwrap();
    assert_eq!(v["name"], serde_json::json!("tc456"));
    assert_eq!(v["tool_name"], serde_json::json!("local/ls"));
}

/// 具名新建：**地址末段即会话 id**；无名目标（挂载根 / 空末段）⇒ `None`（provider 生成）
#[test]
fn session_id_from_new_path_uses_the_address_as_identity() {
    assert_eq!(
        session_id_from_new_path("我的会话").as_deref(),
        Some("我的会话")
    );
    assert_eq!(
        session_id_from_new_path("dir/我的会话").as_deref(),
        Some("我的会话")
    );
    // 不剥扩展名：会话寻址里 `.session` 不是地址的一部分（见函数文档）
    assert_eq!(
        session_id_from_new_path("abc.session").as_deref(),
        Some("abc.session")
    );
    // 无名目标 ⇒ 交给 provider 生成
    assert_eq!(session_id_from_new_path(""), None);
    assert_eq!(session_id_from_new_path("/"), None);
    assert_eq!(session_id_from_new_path("dir/"), None);
    assert_eq!(session_id_from_new_path("   "), None);
}

/// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）、
/// kind / 状态 / 更新时间 / 摘要与 `session_node` 的单点形状同源
#[test]
fn session_node_carries_renderer_ext_and_presentation() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000_000;

    let idle = session_node(&SessionSummary::of(&s), &SessionRuntime::idle(None));
    assert_eq!(idle.name, "abc");
    assert_eq!(idle.effective_ext().as_deref(), Some("session"));
    assert_eq!(idle.kind, PLUGIN_ID_SESSION);
    assert_eq!(idle.status, vdfs::VDFS_STATUS_ACTIVE);
    assert_eq!(idle.updated_at, Some(1_700_000_000_000));
    assert_eq!(idle.access.flags(), "rw", "会话可读可写");
    assert!(!idle.is_dir(), "会话是文档而非目录");

    let busy = session_node(&SessionSummary::of(&s), &SessionRuntime::working());
    assert_eq!(busy.status, vdfs::VDFS_STATUS_WORKING);
}

/// 会话运行态投影：**三个状态**（运行中 / 空闲 / 失败），不是「状态 + `last_failed` 布尔」。
///
/// 这条断言钉住三件事：
/// 1. `failed` 是一个真实状态值（可重试性 = `status == 'failed'`，一处判定）；
/// 2. 「运行中」压过「上次失败」——否则重试期间会同时显示处理中与错误；
/// 3. 中止**不是**失败（状态回空闲，结局只记在 `outcome`，供提示音选音色）。
#[test]
fn session_node_projects_runtime_state() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000;

    let failed = session_node(
        &SessionSummary::of(&s),
        &SessionRuntime::finished(OUTCOME_FAILED, Some("上游 502".to_string()), None),
    );
    assert_eq!(failed.status, vdfs::VDFS_STATUS_FAILED);
    assert_eq!(failed.attributes.get("outcome"), Some(&json!("failed")));
    assert_eq!(failed.attributes.get("error"), Some(&json!("上游 502")));

    let aborted = session_node(
        &SessionSummary::of(&s),
        &SessionRuntime::finished(OUTCOME_ABORTED, None, None),
    );
    assert_eq!(aborted.status, vdfs::VDFS_STATUS_ACTIVE, "中止不是失败");
    assert_eq!(aborted.attributes.get("outcome"), Some(&json!("aborted")));
    assert_eq!(aborted.attributes.get("error"), None, "中止不带错误文案");

    let idle = session_node(&SessionSummary::of(&s), &SessionRuntime::idle(None));
    assert_eq!(idle.status, vdfs::VDFS_STATUS_ACTIVE);
    assert_eq!(
        idle.attributes.get("outcome"),
        None,
        "「还不知道上一轮结局」不写成 null：与「上一轮正常结束」是两件事"
    );
}

/// 会话内部寻址（S6/S16）：`<id>` / `<id>/MEMORY.md` / `<id>/message[/<mid>]` /
/// `<id>/subsession[/<sub>]` / `<id>/workdir[/<rel>]`。
/// 未知区段与越界层级一律 NotFound——不给半通不通的路径留口子。
#[test]
fn vdfs_internal_path_parsing() {
    use VdfsSessionPath::*;
    assert!(matches!(parse_session_path("").unwrap(), Root));
    assert!(matches!(parse_session_path("/").unwrap(), Root));
    assert!(matches!(parse_session_path("abc").unwrap(), Session("abc")));
    // 会话记忆：单个文件，地址用真实文件名
    assert!(matches!(
        parse_session_path("abc/MEMORY.md").unwrap(),
        Memory("abc")
    ));
    assert!(
        parse_session_path("abc/MEMORY.md/deeper").is_err(),
        "记忆是文件，没有更深层级"
    );
    // 转写列表：目录本身与列表项两级
    assert!(matches!(
        parse_session_path("abc/message").unwrap(),
        Messages {
            id: "abc",
            mid: None
        }
    ));
    assert!(matches!(
        parse_session_path("abc/message/m1").unwrap(),
        Messages {
            id: "abc",
            mid: Some("m1")
        }
    ));
    // 收件箱：目录本身与列表项两级，与会话内部其它集合同构
    assert!(matches!(
        parse_session_path("abc/inbox").unwrap(),
        Inbox {
            id: "abc",
            iid: None
        }
    ));
    assert!(matches!(
        parse_session_path("abc/inbox/i1").unwrap(),
        Inbox {
            id: "abc",
            iid: Some("i1")
        }
    ));
    assert!(matches!(
        parse_session_path("abc/subsession").unwrap(),
        SubSessions("abc")
    ));
    assert!(matches!(
        parse_session_path("abc/subsession/s1").unwrap(),
        SubSession {
            id: "abc",
            sub: "s1"
        }
    ));
    assert!(matches!(
        parse_session_path("abc/workdir").unwrap(),
        Workdir { id: "abc", rel: "" }
    ));
    assert!(matches!(
        parse_session_path("abc/workdir/src/lib.rs").unwrap(),
        Workdir {
            id: "abc",
            rel: "src/lib.rs"
        }
    ));
    // 未知区段、列表项越界层级 → NotFound
    assert!(parse_session_path("abc/nope").is_err());
    assert!(parse_session_path("abc/message/m1/deeper").is_err());
    assert!(parse_session_path("abc/inbox/i1/deeper").is_err());
    assert!(parse_session_path("abc/subsession/s1/deeper").is_err());
}

/// 收件箱写入正文的两种形状：以 `{` 起头 = 结构化字段子集；其余 = 纯文本。
///
/// 判据必须**只由首字符**决定：改回"能不能解析成 JSON"，一个手滑的 `{content}`
/// 就会被当成正文静默发出去（调用方以为自己在写结构化字段）。
#[test]
fn parse_inbox_message_distinguishes_shape_by_first_char() {
    // 纯文本：整段就是正文，角色 / 类型 / 状态都是"待发用户消息"
    let m = parse_inbox_message("帮我看一下这个 bug").unwrap();
    assert_eq!(m.content.unwrap().to_text(), "帮我看一下这个 bug");
    assert_eq!(m.role, Some(cm::MessageRole::User));
    assert_eq!(m.msg_type, Some(cm::MessageType::Text));
    assert_eq!(m.status, Some(cm::MessageStatus::Completed));

    // 结构化：字段子集直接用，缺省项照补
    let m = parse_inbox_message(r#"{"content":"结构化正文","role":"user"}"#).unwrap();
    assert_eq!(m.content.unwrap().to_text(), "结构化正文");
    assert_eq!(m.msg_type, Some(cm::MessageType::Text), "未给的字段走缺省");
    assert_eq!(m.id, "", "`id` 缺省是空串占位——身份由地址 / 入队统一落定");
    // 写体给了 id 就先用它（地址若也给，则以地址为准——见 `enqueue_inbox`）
    let m = parse_inbox_message(r#"{"id":"from-body","content":"x"}"#).unwrap();
    assert_eq!(m.id, "from-body");

    // 声明了 JSON 就必须是合法 JSON 对象——不静默降级成"把这串当正文"
    assert!(parse_inbox_message(r#"{"content":}"#).is_err());
    assert!(parse_inbox_message("{不是 JSON对象}").is_err());
}

/// 收件箱条目：节点形状与消息节点同源，只靠 `kind` 与摘要前缀区分"待发 / 已发生"。
#[test]
fn inbox_item_node_reuses_message_shape() {
    let item = crate::plugins::session::active::InboxItem {
        id: "i1".into(),
        message: cm::ChatMessage {
            id: "i1".into(),
            role: Some(cm::MessageRole::User),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text("待发的这句话".into())),
            status: Some(cm::MessageStatus::Completed),
            timestamp: Some(1_700_000_000_000),
            ..Default::default()
        },
        params: Default::default(),
        workdir: None,
    };
    let n = inbox_item_node(&item);
    assert_eq!(n.name, "i1", "条目 id 就是地址末段");
    assert_eq!(n.kind, KIND_INBOX);
    assert_eq!(
        n.effective_ext().as_deref(),
        Some("message"),
        "它就是一条消息"
    );
    assert_eq!(n.title, "用户");
    assert_eq!(n.description.as_deref(), Some("待消费 · 待发的这句话"));
    assert_eq!(n.updated_at, Some(1_700_000_000_000));
}

/// 会话内部的虚拟子项：转写列表、收件箱与子会话恒在，记忆是**文件**且恒在，
/// 工作目录按会话是否声明 workdir 出现。
#[test]
fn vdfs_internal_dirs_conditional() {
    // 有作用域才有节点名（无作用域 = 没有文件 = 没有名字，共享实现如实返回空名）。
    // 这里按生产路径的形态构造：给一个真实落位，节点名就是它的文件名。
    let memory = || {
        let path = std::path::PathBuf::from("abc")
            .join(crate::plugins::session::memory::SESSION_MEMORY_FILE);
        crate::providers::memory::MemoryFile::new(Some(path), 1024, 256).node(
            &crate::providers::memory::MemoryNodeSpec {
                title: "会话记忆",
                kind: PLUGIN_ID_SESSION,
                description: "d",
            },
        )
    };

    let without = internal_dirs(false, memory());
    assert_eq!(without.len(), 4);
    assert_eq!(without[0].name, SEG_MESSAGES, "转写列表恒在（会话的本体）");
    assert_eq!(
        without[0].name, "message",
        "路径段一律 ASCII（地址要能安全地进 URL / 命令行 / 日志）"
    );
    assert_eq!(without[0].title, TITLE_MESSAGES, "展示名才是中文");
    // 段名与展示名不是一回事，**标识**由 kind 承担：消费者按 kind 发现转写列表，
    // 不必把段名写进自己的地址模板（前端镜像守卫 X-002 校验的就是这个词）
    assert_eq!(
        without[0].kind, KIND_MESSAGES,
        "转写列表的 kind 是稳定协议词，不随段名 / 展示名变化"
    );
    assert_eq!(without[1].name, SEG_INBOX, "收件箱恒在（消息的入队面）");
    assert_eq!(without[1].title, TITLE_INBOX, "展示名才是中文");
    assert_eq!(
        without[1].kind, KIND_INBOX,
        "收件箱的 kind 是稳定协议词（与会话转写消息区分开）"
    );
    assert_eq!(without[2].name, workdir::SEG_SUB_SESSIONS);
    assert_eq!(
        without[3].name,
        crate::plugins::session::memory::SESSION_MEMORY_FILE,
        "记忆恒在——它本来就是会话的一部分"
    );
    assert!(!without[3].is_dir(), "记忆是文件，不是目录");
    assert!(without[3].access.write, "记忆可写（模型与用户共用这一份）");
    assert!(
        without[..3].iter().all(|n| n.is_dir() && !n.access.write),
        "三个目录区段都是只读目录"
    );

    let with = internal_dirs(true, memory());
    assert_eq!(with.len(), 5);
    assert_eq!(with[4].name, workdir::SEG_WORKDIR);
    assert!(
        with[4].is_dir() && !with[4].access.write,
        "工作目录是只读目录（不提供新建）"
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
    assert_eq!(
        n.status, "completed",
        "消息状态**直通**，不再坍缩成会话态 active——\
         坍缩会把 completed / failed 两种结局压成同一个值，消费端再也猜不回来"
    );
    // 没写 status 的历史消息（老数据）默认已结束：缺省不等于「未开始」
    m.status = None;
    assert_eq!(message_node(&m).status, "completed");
    m.status = Some(cm::MessageStatus::Completed);
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
        ids(&transcript_window(&msgs, Some(2), Some("x/s/message/t3"))),
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

/// 地址的「拼」与「解」互逆——改地址方案时漏改一边会被这条挡住
#[test]
fn message_path_round_trips_through_parser() {
    let p = message_path("abc", "m1");
    assert_eq!(p, "abc/message/m1");
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

    let n = session_node(&SessionSummary::of(&s), &SessionRuntime::idle(None));
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

    // 运行态由 status 承载（机制口径，不另设 is_working 字段）：
    // 空闲 / 运行中 / 上次失败是三个**并列的状态值**，不是布尔 + 标志位
    assert_eq!(n.status, vdfs::VDFS_STATUS_ACTIVE);
    assert_eq!(
        session_node(&SessionSummary::of(&s), &SessionRuntime::working()).status,
        vdfs::VDFS_STATUS_WORKING
    );
    assert_eq!(
        session_node(
            &SessionSummary::of(&s),
            &SessionRuntime::finished(OUTCOME_FAILED, Some("boom".into()), None)
        )
        .status,
        vdfs::VDFS_STATUS_FAILED,
        "以错误结束是一个独立状态：空闲但上次失败"
    );
    // 正常收尾 → 回到空闲（不是第三个「已完成」态：会话是长驻容器，不是一次性任务）
    assert_eq!(
        session_node(
            &SessionSummary::of(&s),
            &SessionRuntime::finished(OUTCOME_COMPLETED, None, None)
        )
        .status,
        vdfs::VDFS_STATUS_ACTIVE
    );
}

/// 会话内容（VDFS `read`）：转写全文 + 元数据，JSON
#[test]
fn vdfs_session_content_is_json() {
    let mut s = Session::new("abc");
    s.updated_at = 1_700_000_000_000;
    let c = session_content(&s, Vec::new()).unwrap();
    let text = c.text.as_deref().unwrap_or("");
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["id"], "abc");
    assert!(v.get("messages").is_some(), "聊天转写随内容下发");
}

/// 会话内容必须**叠加在途消息**，与转写列表同源。
///
/// 前端 `loadMessages` 读的是叶子（`fetchTranscript` → `readVdfs(vdfsSessionAddr)`），
/// 不是专用读协议（`session/get_messages` 已于 2026-09-23 退役）。若叶子只序列化存储，
/// Turn 运行中切走再切回就会
/// 看不到正在跑的那一轮——它在 `persist_messages` 落库之前只存在于在途缓冲里。
#[test]
fn vdfs_session_content_overlays_live_messages() {
    let mut s = Session::new("abc");
    s.messages = vec![msg("stored-1", None)];

    let c = session_content(&s, vec![msg("live-1", None)]).unwrap();
    let v: Value = serde_json::from_str(c.text.as_deref().unwrap_or("")).unwrap();
    let ids: Vec<&str> = v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["stored-1", "live-1"], "落库在前，在途在后");

    // 同 id：在途版本胜出（它更新），但 `seq` 从落库版本继承——否则同一条消息
    // 会以「有 seq / 无 seq」两种形态排到列表的两个位置。
    let mut stored = msg("same", Some(7));
    stored.content = Some(cm::MessageContent::Text("旧".into()));
    let mut live = msg("same", None);
    live.content = Some(cm::MessageContent::Text("新".into()));
    let mut s2 = Session::new("abc");
    s2.messages = vec![stored];
    let c = session_content(&s2, vec![live]).unwrap();
    let v: Value = serde_json::from_str(c.text.as_deref().unwrap_or("")).unwrap();
    let only = &v["messages"].as_array().unwrap()[0];
    assert_eq!(only["content"], "新");
    assert_eq!(only["seq"], 7, "顺序锚点只由存储分配，在途副本必须继承");
}
