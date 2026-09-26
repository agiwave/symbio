//! VDFS **节点构造 / 路径模型 / 消息投影**（自 `plugin.rs` 原样搬移，拆文件不拆行为）。
//!
//! 三组**纯函数**，都不持有 `self`：
//! - **路径模型**：[`VdfsSessionPath`] / [`parse_session_path`] / [`SEG_MESSAGES`] /
//!   [`SEG_INBOX`] / [`message_dir_path`] / [`message_path`] / [`inbox_item_path`] /
//!   [`internal_dirs`] / [`session_id_from_new_path`]
//! - **节点构造**：[`session_node`] / [`message_node`] / [`inbox_dir_node`] /
//!   [`inbox_item_node`] / [`transcript_window`] / [`window_params`] /
//!   `MAX_PARENT_STEPS` / `cursor_id`
//! - **消息投影**：`message_label` /
//!   `message_status` / `message_preview` / [`message_text`] / [`ordered`] /
//!   [`overlay_live`] / [`message_of`] / [`session_content`]
//!
//! 可见性：被 `plugin.rs` / `vdfs_provider.rs` / `plugin.test.rs` 取用的项标
//! `pub(crate)`，由 `plugin.rs` 统一重导出；只在本文件内部使用的保持私有。

use super::*;

/// 会话的**运行态**——非持久化，随进程与当前请求变化。
///
/// 它是「会话节点状态」的唯一来源：节点 `status` 与 `attributes.outcome` /
/// `attributes.error` 全部由它投影，因此「会话忙不忙 / 上一轮怎么结束的」
/// 在任何消费端都只有一处判据（见 `session/docs/node-state-streaming.md` §2.3）。
///
/// 为什么不是一个 `is_working: bool` 参数：布尔只能表达两态，而会话需要三态
/// （`working` / `active` / `failed`），且 `failed` 还要带上错误文案。
/// 让调用方各自拼状态串，等于把判据复制到每个调用点。
pub(crate) struct SessionRuntime {
    /// 正在处理一轮交互
    pub working: bool,
    /// 上一轮结局：`completed` / `aborted` / `failed`（`None` = 本进程内尚未跑完过一轮）
    pub outcome: Option<String>,
    /// 上一轮的错误短消息（仅 `outcome == failed` 时有意义）
    pub error: Option<String>,
    /// 会话级告警（可恢复）：持久化失败 / 长度截断 / 工具轮次上限。
    /// 与 `error` 的分界：告警不改变运行态，会话照常运行；新一轮开始即清除。
    pub warning: Option<String>,
}

/// 上一轮结局的词表：正常结束 / 用户中止 / 以错误结束。
///
/// 定义在会话自己的协议词表（`super::words`）——会话节点 `attributes.outcome`
/// 是**跨前端**的线上契约，进程内消费者（CLI）也要按同一批字面量判读，
/// 因此各取用方直接从词表导入，不另立一份。
use super::words::OUTCOME_FAILED;

impl SessionRuntime {
    /// 空闲：没在跑，也没有已知的上一轮结局
    pub(crate) fn idle(warning: Option<String>) -> Self {
        Self {
            working: false,
            outcome: None,
            error: None,
            warning,
        }
    }

    /// 运行中：新一轮开始，上一轮的结局与告警随之作废（否则失败角标/告警条会残留）
    pub(crate) fn working() -> Self {
        Self {
            working: true,
            outcome: None,
            error: None,
            warning: None,
        }
    }

    /// 由上一轮结局构造（`failed` 时带错误文案；告警原样保留——
    /// 「以告警收尾的一轮」在结束后仍应能看到那条告警）
    pub(crate) fn finished(outcome: &str, error: Option<String>, warning: Option<String>) -> Self {
        Self {
            working: false,
            outcome: Some(outcome.to_string()),
            error: if outcome == OUTCOME_FAILED {
                error
            } else {
                None
            },
            warning,
        }
    }

    /// 从活跃会话状态（`is_working` + 结局 + 错误 + 告警）投影运行态。
    ///
    /// **唯一入口**：把「运行中时结局一律作废」这条规则收在一处——否则
    /// 「正在跑却带着上次错误」这种非法组合会在每个调用点各拼一次，而它的
    /// 表现是静默的（前端同时显示"处理中"与"错误"）。
    pub(crate) fn from_state(
        working: bool,
        outcome: Option<String>,
        error: Option<String>,
        warning: Option<String>,
    ) -> Self {
        if working {
            return Self::working();
        }
        match outcome.as_deref() {
            Some(o) => Self::finished(o, error, warning),
            None => Self::idle(warning),
        }
    }

    /// 节点状态：运行中 > 上次失败 > 空闲。
    ///
    /// 顺序有意义——「正在跑」永远压过「上次失败了」：否则重试期间会同时
    /// 显示"处理中"与"错误"。
    fn status(&self) -> &'static str {
        if self.working {
            return vdfs::VDFS_STATUS_WORKING;
        }
        if self.outcome.as_deref() == Some(OUTCOME_FAILED) {
            return vdfs::VDFS_STATUS_FAILED;
        }
        vdfs::VDFS_STATUS_ACTIVE
    }
}

/// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）。
///
/// 入参是 [`SessionSummary`] 而非 `Session`——**清单路径根本不持有消息**，
/// 于是「节点又去碰消息」在类型上就写不出来。标题 / 摘要 / 元信息标签都是
/// 存储层保存时算好的**投影**（`SessionSummary::of`），呈现口径因此单点。
///
/// 另在 `attributes` 上挂载会话清单所需字段（`message_count` / `metadata` /
/// `meta_tags`）——它们是**场景数据**，VDFS 只透传；会话清单由此可直接用
/// `vdfs/list` 一次取全。
///
/// 运行态由 [`SessionRuntime`] 投影：`status` 是运行态本身，`outcome` / `error`
/// 是它的两个场景属性（消费者据此选提示音音色、渲染会话级错误条）。
pub(crate) fn session_node(s: &SessionSummary, rt: &SessionRuntime) -> vdfs::VdfsNode {
    let mut n = vdfs::VdfsNode::file(&s.id, s.title.clone(), vdfs::VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_ID_SESSION.to_string();
    n.ext = Some(EXT_SESSION.to_string());
    n.status = rt.status().to_string();
    n.updated_at = Some(s.updated_at);
    n.description = s.summary.clone();
    let _ = n
        .attributes
        .insert("message_count".to_string(), json!(s.message_count));
    let _ = n
        .attributes
        .insert("metadata".to_string(), s.metadata.clone());
    let _ = n
        .attributes
        .insert("meta_tags".to_string(), json!(s.meta_tags));
    // 上一轮结局：只在知道时出现（不写成 null——「还不知道」与「上一轮正常结束」
    // 是两件事，不该在线上形状里混为一谈）
    if let Some(outcome) = &rt.outcome {
        let _ = n.attributes.insert("outcome".to_string(), json!(outcome));
    }
    if let Some(error) = &rt.error {
        let _ = n.attributes.insert("error".to_string(), json!(error));
    }
    if let Some(warning) = &rt.warning {
        let _ = n.attributes.insert("warning".to_string(), json!(warning));
    }
    n
}

// ==================== 有界列表（VDFS 调用级参数袋） ====================
//
// 「只取一页」不是会话专有需求——任何清单都会有这一天。所以它不是一个新接口，
// 而是 `vdfs/list` 的**调用级参数**：谁传谁生效，不传就与从前逐字节一致
// （`VdfsProvider::list` 的签名因此不必改动，其它 provider 一行都不用动）。

/// 从调用级参数袋里取窗口：`limit`（条数，名义值）与 `before`（游标 = 上一页
/// 最后一个条目的地址）。
pub(crate) fn window_params(ctx: &vdfs::VdfsContext) -> (Option<u32>, Option<&str>) {
    (
        ctx.param_as::<u32>(VDFS_PARAM_LIMIT),
        ctx.param_str(VDFS_PARAM_BEFORE),
    )
}

/// 沿 `parent_id` 上溯的步数上限（防御成环；正常转写远小于此）
const MAX_PARENT_STEPS: usize = 64;

/// 转写的有界窗口 —— **根节点为计量单位**，且**父节点闭合**。
///
/// ## 为什么计量单位是「根」而不是「条」
///
/// 一个 Turn = 一个根消息 + 它的全部后代（reason / tool_call / 文本分块）。
/// 按条数截断会把 Turn 劈成两半：前端拿到 reason 却拿不到它属于哪一轮，
/// 树就拼不起来。所以 `limit` 是**名义值**——实际返回的条数恒 ≥ `limit`。
///
/// ## 父节点闭合
///
/// 只要某个根被选中，它的**全部**后代都在窗口里；反过来，窗口里不会出现在
/// 窗口外的父节点（否则同样拼不成树）。
///
/// `before` 是上一页最后一条的地址（消息 id 或 `<…>/<id>`）；它会被归到自己的
/// 根，从那个根**往前**再取 `limit` 个根。找不到游标（已删 / 已到末尾）返回空页，
/// 让调用方自然收敛，不报错。
pub(crate) fn transcript_window(
    msgs: &[cm::ChatMessage],
    limit: Option<u32>,
    before: Option<&str>,
) -> Vec<cm::ChatMessage> {
    // 没给窗口参数 = 全量（前端流式期间要的就是完整列表）
    if limit.is_none() && before.is_none() {
        return msgs.to_vec();
    }

    let index: HashMap<&str, usize> = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.as_str(), i))
        .collect();

    // 每条消息的**根**：沿 parent_id 上溯；parent 缺失或不在列表里 ⇒ 自己即根
    let root_of: Vec<usize> = msgs
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let mut cur = i;
            for _ in 0..MAX_PARENT_STEPS {
                let parent = match msgs[cur].parent_id.as_deref() {
                    Some(p) if !p.is_empty() => p,
                    _ => break,
                };
                match index.get(parent) {
                    Some(&pi) if pi != cur => cur = pi,
                    _ => break,
                }
            }
            cur
        })
        .collect();

    // 根的出现顺序（去重，保留首次出现序）
    let mut roots: Vec<usize> = Vec::new();
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for &r in &root_of {
        if seen.insert(r) {
            roots.push(r);
        }
    }

    let end = match before.and_then(|b| cursor_id(b)).and_then(|b| index.get(b)) {
        Some(&i) => roots
            .iter()
            .position(|&r| r == root_of[i])
            .unwrap_or(roots.len()),
        None => roots.len(),
    };
    let start = match limit {
        Some(n) => end.saturating_sub(n as usize),
        None => 0,
    };

    let keep: std::collections::HashSet<usize> = roots[start..end].iter().copied().collect();
    msgs.iter()
        .zip(&root_of)
        .filter(|(_, r)| keep.contains(r))
        .map(|(m, _)| m.clone())
        .collect()
}

/// 游标 → 消息 id：游标可以是裸 id，也可以是 `<…>/<id>` 的地址形式
fn cursor_id(before: &str) -> Option<&str> {
    let b = before.trim_end_matches('/');
    match b.rsplit('/').next() {
        Some(id) if !id.is_empty() => Some(id),
        _ => None,
    }
}

// ==================== VDFS：会话内部寻址 ====================
//
// 会话在 VDFS 上**保持叶子**（`ext = session`，点击进聊天详情，语义不变）；
// 其内部结构（记忆 / 转写 / 子会话 / 工作目录树）作为**会话同名目录**挂在会话之下：
//
//   <id>                  → 会话叶子（聊天详情）
//   <id>/AGENTS.md         → 会话记忆（**单个文件**，读写；见 `super::super::memory`）
//   <id>/message[/<mid>]   → 转写列表 / 单条消息（**列表项**）
//   <id>/inbox[/<iid>]     → 收件箱（待消费用户消息，写即入队）
//   <id>/subsession[/<sub>]→ 子会话清单 / 单个子会话（查看 · 删除）
//   <id>/workdir[/<rel>]   → 工作目录树（文件可查看 / 编辑）
//
// **路径段一律 ASCII，中文只出现在 `title`（展示名）上**：地址要能安全地进 URL、
// 命令行、日志与文件名，不受编码 / 输入法 / 大小写折叠的影响。标识由 `kind`
// 承担（见 [`messages_dir_node`]），消费者按 `kind` 发现一段，不把段名写进模板。
//
// 场景实现复用 `workdir` 与子会话清单——VDFS 是这些能力的**唯一入口**，不再有
// 并行的「容器页」路由。

// 转写路径段 [`SEG_MESSAGES`] 与节点逆投影 [`message_of_node`] 已上移到
// `symbio_core::schemas::session::chat_message`——它们描述的是**跨插件契约**
// （agent 的子会话转播桥也要用），不是本插件的私事。本模块继续按原名使用。
use crate::symbio_core::schemas::session::chat_message::SEG_MESSAGES;

/// 转写列表的**展示名**（`title`）。**只影响 UI**，不参与寻址。
pub(crate) const TITLE_MESSAGES: &str = "消息";

/// 转写列表目录节点（`list` 与 `stat` 共用同一份形状）。
///
/// `name` = 路径段（[`SEG_MESSAGES`]，ASCII）、`title` = 展示名（[`TITLE_MESSAGES`]）。
/// `kind` 承担对外标识：消费者按它发现这一段，不必把段名写进自己的地址模板
/// （见 `tauri/src/services/vdfsScheme.ts::resolveMessagesSeg`）。
pub(crate) fn messages_dir_node() -> vdfs::VdfsNode {
    let mut node = vdfs::VdfsNode::dir(SEG_MESSAGES, TITLE_MESSAGES, vdfs::VdfsAccess::LIST);
    node.kind = KIND_MESSAGES.to_string();
    node
}

/// 转写列表本身的 provider 子树内路径（`<id>/message`）。
///
/// 与 [`message_path`] 同源：列表与列表项是同一地址方案的两级。
///
/// **它不再是变更的落点**：变更落在**节点自身**（[`message_path`]），目录只服务
/// `list` 与 `action`（截断 / 清空）。因此本函数只剩测试与文档在用。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn message_dir_path(session_id: &str) -> String {
    format!("{session_id}/{SEG_MESSAGES}")
}

/// 会话内部：**收件箱**的路径段（ASCII，进地址）。
///
/// 收件箱是**待消费的用户消息队列**：往里写一条 = 入队，被消费（跑一轮）时消息
/// 才落库进转写。因此它同时是「跨空间发消息」的入口——子智能体空间没有调用方
/// 替它调 `chat/send`，它的会话由自己消费这个队列而自驱动（见 ADR-026）。
///
/// 与会话内部其它集合同构：`<sid>/<集合段>/<项 id>`，身份就是末段。
pub(crate) const SEG_INBOX: &str = "inbox";

/// 收件箱的**展示名**（`title`）。只影响 UI，不参与寻址。
pub(crate) const TITLE_INBOX: &str = "收件箱";

/// 收件箱目录节点（`list` 与 `stat` 共用同一份形状）
pub(crate) fn inbox_dir_node() -> vdfs::VdfsNode {
    let mut node = vdfs::VdfsNode::dir(SEG_INBOX, TITLE_INBOX, vdfs::VdfsAccess::LIST);
    node.kind = KIND_INBOX.to_string();
    node
}

/// 单条收件箱条目（`<sid>/inbox/<iid>`）的 provider 子树内路径
pub(crate) fn inbox_item_path(session_id: &str, iid: &str) -> String {
    format!("{session_id}/{SEG_INBOX}/{iid}")
}

/// 收件箱写入的**正文** → 一条待消费的用户消息。
///
/// ## 两种形状，判别式只有一条
///
/// - **以 `{` 起头** ⇒ 它声明自己是 JSON 对象 —— 必须解析成 [`cm::ChatMessage`]
///   的**字段子集**，解析失败即报错（不静默降级成"把这串 JSON 当正文"）；
/// - **其余** ⇒ 整段正文就是消息文本（发一条消息的自然写法）。
///
/// 判据取"以 `{` 开头"而不是"能不能解析成 JSON"：后者会让一个手滑的
/// `{content}` 被当成正文静默发出去，而调用方以为自己在写结构化字段。
///
/// 缺省全由消息自身的约定补：`role` = `user`、`type` = `text`、`status` =
/// `completed`（它是一条**待发**消息，不是流式中的消息）。`id` 缺失时补**空串
/// 占位**，由 [`SessionPlugin::enqueue_inbox`] 落到真正的条目 id 上——身份有三个
/// 来源（地址末段 > 写体里的 `id` > provider 生成），判别收在一处，不在这里各判一次。
pub(crate) fn parse_inbox_message(raw: &str) -> vdfs::VdfsResult<cm::ChatMessage> {
    let body = raw.trim();
    if !body.starts_with('{') {
        return Ok(cm::ChatMessage {
            role: Some(cm::MessageRole::User),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text(raw.to_string())),
            status: Some(cm::MessageStatus::Completed),
            timestamp: Some(crate::symbio_core::clock_now_ms()),
            ..Default::default()
        });
    }
    let mut value: Value = serde_json::from_str(body).map_err(|e| {
        vdfs::VdfsError::invalid(format!(
            "收件箱条目需要合法 JSON（ChatMessage 字段子集）：{e}"
        ))
    })?;
    let Some(obj) = value.as_object_mut() else {
        return Err(vdfs::VdfsError::invalid(
            "收件箱条目需要 JSON 对象（ChatMessage 字段子集）",
        ));
    };
    // `id` 在 `ChatMessage` 里是必填字段，但**身份由地址给**：缺它时补一个空串
    // 占位，由 `enqueue_inbox` 统一落到条目 id 上——不这样做，最自然的那份
    // 写体（`{"content":"..."}`）会先一步被 serde 拒掉，而调用方被迫把地址里
    // 已有的信息再抄一遍（与消息补丁的处理同源，见 `vdfs_provider` 的 `write`）。
    obj.entry("id".to_string())
        .or_insert_with(|| Value::String(String::new()));
    let mut message: cm::ChatMessage = serde_json::from_value(value).map_err(|e| {
        vdfs::VdfsError::invalid(format!(
            "收件箱条目需要合法 JSON（ChatMessage 字段子集）：{e}"
        ))
    })?;
    // 缺省补齐（见上：只补"不补就不可用"的那三个，不覆盖来者指定的值）
    message.role.get_or_insert(cm::MessageRole::User);
    message.msg_type.get_or_insert(cm::MessageType::Text);
    message.status = Some(cm::MessageStatus::Completed);
    Ok(message)
}

/// 收件箱条目 → VDFS 节点。
///
/// 形状**就是消息节点的形状**（[`message_node`]）：条目本来就是一条用户消息，
/// 再造一套"待发消息"的字段只会让同一件事有两种读法。差别只有两处，且都在
/// `kind` / 说明上——`kind = inbox` 让消费者分清"待发"与"已发生"，摘要前缀
/// 让列表里一眼看出这条还没被消费。
pub(crate) fn inbox_item_node(item: &InboxItem) -> vdfs::VdfsNode {
    let mut n = message_node(&item.message);
    n.kind = KIND_INBOX.to_string();
    n.description = Some(match n.description {
        Some(d) => format!("待消费 · {d}"),
        None => "待消费".to_string(),
    });
    n
}

/// 单条消息的 provider 子树内路径（`<id>/message/<mid>`）。
///
/// 与 [`parse_session_path`] 互逆，因此与它同处——地址的「拼」与「解」必须同源，
/// 分开写就会在改地址方案时漏改一边。
///
/// ## 它现在同时是**变更的落点**
///
/// 信封的 `path` 恒为**被变更节点自身的地址**（见 `Transcript::emit`）：消息的
/// 实时面与历史面因此共用这一个形状，消费端拿到的 `path` 就是它能直接 `read` /
/// `stat` 的地址——不需要「拿载荷里的 id 反推地址」。
///
/// 会话是**容器**，其下是若干并列的**集合**（消息 / 子会话 / 记忆 / 工作目录，
/// 后续还会有任务列表、请求队列……）：`<sid>/<集合段>/<项 id>` 是统一形状，
/// 新增一类集合只需在 [`internal_dirs`] 里声明一个段，机制与消费端的判据都不变。
pub(crate) fn message_path(session_id: &str, mid: &str) -> String {
    format!("{session_id}/{SEG_MESSAGES}/{mid}")
}

/// 会话挂载点内的路径解析结果
pub(crate) enum VdfsSessionPath<'a> {
    /// 挂载根 = 会话清单
    Root,
    /// `<id>`：单个会话（叶子）
    Session(&'a str),
    /// `<id>/AGENTS.md`：会话记忆（**单个文件**，可读写）
    Memory(&'a str),
    /// `<id>/message[/<mid>]`：转写列表 / 单条消息；`mid` 空 = 列表本身
    Messages { id: &'a str, mid: Option<&'a str> },
    /// `<id>/inbox[/<iid>]`：收件箱（待消费的用户消息）/ 单条待消费消息
    Inbox { id: &'a str, iid: Option<&'a str> },
    /// `<id>/subsession`：子会话清单
    SubSessions(&'a str),
    /// `<id>/subsession/<sub>`：单个子会话
    SubSession { id: &'a str, sub: &'a str },
    /// `<id>/workdir[/<rel>]`：工作目录树；`rel` 空 = 工作目录根
    Workdir { id: &'a str, rel: &'a str },
}

/// 解析会话挂载点内的相对路径（首段 = 会话 id，次段 = 内部区段）。
pub(crate) fn parse_session_path(path: &str) -> vdfs::VdfsResult<VdfsSessionPath<'_>> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return Ok(VdfsSessionPath::Root);
    }
    let (id, rest) = match p.split_once('/') {
        Some((id, rest)) => (id, rest),
        None => return Ok(VdfsSessionPath::Session(p)),
    };
    let (seg, sub) = match rest.split_once('/') {
        Some((seg, sub)) => (seg, Some(sub)),
        None => (rest, None),
    };
    let not_found = || vdfs::VdfsError::not_found(format!("会话内部不存在该路径：{path}"));
    match seg {
        // 记忆是**单个文件**：地址用真实文件名（`AGENTS.md`），没有更深层级
        crate::symbio_core::MEMORY_AGENTS_FILE => match sub {
            None => Ok(VdfsSessionPath::Memory(id)),
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "记忆是文件，没有更深层级：{path}"
            ))),
        },
        SEG_MESSAGES => match sub {
            None => Ok(VdfsSessionPath::Messages { id, mid: None }),
            Some(mid) if !mid.is_empty() && !mid.contains('/') => {
                Ok(VdfsSessionPath::Messages { id, mid: Some(mid) })
            }
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "消息是列表项，没有更深层级：{path}"
            ))),
        },
        SEG_INBOX => match sub {
            None => Ok(VdfsSessionPath::Inbox { id, iid: None }),
            Some(iid) if !iid.is_empty() && !iid.contains('/') => {
                Ok(VdfsSessionPath::Inbox { id, iid: Some(iid) })
            }
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "收件箱条目是列表项，没有更深层级：{path}"
            ))),
        },
        super::super::workdir::SEG_SUB_SESSIONS => match sub {
            None => Ok(VdfsSessionPath::SubSessions(id)),
            Some(sub) if !sub.is_empty() && !sub.contains('/') => {
                Ok(VdfsSessionPath::SubSession { id, sub })
            }
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "子会话是叶子节点，没有更深层级：{path}"
            ))),
        },
        super::super::workdir::SEG_WORKDIR => Ok(VdfsSessionPath::Workdir {
            id,
            rel: sub.unwrap_or(""),
        }),
        _ => Err(not_found()),
    }
}

/// 会话内部的虚拟子项（工作目录按会话是否声明 workdir 决定是否出现）。
///
/// `memory` 由调用方构造好传入（形状由内核 [`MemoryFile::node`] 产出，
/// `list` 与 `stat` 因此共用同一份形状）；它是个**文件**，与各目录并列——
/// 记忆本来就是会话的一部分，不该另开一条寻址。
///
/// 每个目录节点各由**自己的段所属模块**构造（[`messages_dir_node`] /
/// [`inbox_dir_node`] / `workdir::sub_sessions_dir_node` / `workdir::workdir_dir_node`）：
/// 段名（ASCII，进地址）与展示名（中文，只进 UI）的配对因此与段本身同处，
/// 新增一类集合只需在这里多一项，不必在两处同步改字符串。
pub(crate) fn internal_dirs(has_workdir: bool, memory: vdfs::VdfsNode) -> Vec<vdfs::VdfsNode> {
    let mut out = vec![
        messages_dir_node(),
        inbox_dir_node(),
        super::super::workdir::sub_sessions_dir_node(),
        memory,
    ];
    if has_workdir {
        out.push(super::super::workdir::workdir_dir_node());
    }
    out
}

// ==================== VDFS：转写列表项 ====================
//
// 呈现的分工只有一条：**正文进内容，结构进 `attributes`**。
// - `read` 取到的是这条消息的**正文**——流式追加的正是它，因此追加型变更的增量
//   可以直接拼在尾部，消费者无需为每个片段重读整条消息；
// - 角色 / 类型 / 状态 / 顺序 / 归属等**结构字段**放 `attributes`——它们是场景
//   数据，VDFS 只透传，渲染器按 `ext = message` 自行取用。
//
// 消息在 VDFS 上是**只读列表项**：发言由聊天协议承载（一次发言触发一整轮编排，
// 不是一次文件写入），VDFS 只做「读同一份数据」，因此不存在两条写路径。

/// 消息节点的标题：角色（工具调用补上工具名，否则一屏全是「助手」）
fn message_label(m: &cm::ChatMessage) -> String {
    let role = match m.role {
        Some(cm::MessageRole::User) => "用户",
        Some(cm::MessageRole::Assistant) => "助手",
        Some(cm::MessageRole::Tool) => "工具",
        Some(cm::MessageRole::System) => "系统",
        None => "消息",
    };
    match (m.msg_type.as_ref(), m.name.as_deref()) {
        (Some(cm::MessageType::ToolCall), Some(name)) if !name.is_empty() => {
            format!("{role} · {name}")
        }
        (Some(cm::MessageType::ToolCall), _) => format!("{role} · 工具调用"),
        _ => role.to_string(),
    }
}

/// 消息状态词——**就是 `MessageStatus` 自己的状态词**（[`cm::MessageStatus::as_str`]）。
///
/// ## 为什么是原样透传
///
/// 节点状态只有一套词汇表，不为场景再造一套。**映射**在这里是有害的：把
/// `Completed` 与「未标注」都映射成 VDFS 的常规状态词 `active`，等于让消费端必须把
/// `active` **猜回** `completed`——一次信息丢失加一次还原，任何一端改口径都会静默错。
/// 原样透传后，会话节点的 `active`（空闲）与消息的 `completed`（已结束）不再撞名，
/// 它们本来就是两件事。
fn message_status(m: &cm::ChatMessage) -> &'static str {
    match m.status.as_ref() {
        Some(s) => s.as_str(),
        // 未标注 = 已结束（流式中的节点一定带 `Streaming`，故 `None` 不可能是"进行中"）
        None => cm::MessageStatus::Completed.as_str(),
    }
}

/// 消息摘要（首行、压空白、限长）——列表里的一行预览
fn message_preview(m: &cm::ChatMessage) -> Option<String> {
    let text = m.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    const MAX: usize = 60;
    if flat.chars().count() <= MAX {
        return Some(flat);
    }
    Some(format!("{}…", flat.chars().take(MAX).collect::<String>()))
}

/// 单条消息 → VDFS 节点（**列表项**）
pub(crate) fn message_node(m: &cm::ChatMessage) -> vdfs::VdfsNode {
    let mut n = vdfs::VdfsNode::file(&m.id, message_label(m), vdfs::VdfsAccess::READ);
    n.ext = Some(EXT_MESSAGE.to_string());
    n.status = message_status(m).to_string();
    n.updated_at = m.timestamp;
    n.description = message_preview(m);
    for (k, v) in [
        ("role", json!(m.role)),
        ("type", json!(m.msg_type)),
        // 工具名等**结构字段**：进程内消费者（子会话转播 / CLI）靠它还原消息，
        // 节点标题里虽也带工具名，但那是展示文案，不可当数据读。
        //
        // ⚠️ 键名必须是 `tool_name` 而不是 `name`：`VdfsNode` 结构体自身的
        // `name` 字段承载**节点 id**，而 attributes 经 `#[serde(flatten)]`
        // 序列化到顶层——同名键会让 attributes 的值覆盖结构体的 id
        // （文本 / Turn / Reasoning 节点的工具名是 `null`，序列化后节点 id
        // 变成 null，前端按 `node.name` 寻址的整条实时链路随之瘫痪）。
        ("tool_name", json!(m.name)),
        ("parent_id", json!(m.parent_id)),
        // ToolCall 的 wire id（provider 原始 tool_call_id；节点 id 才是本节点的地址）
        ("tool_call_id", json!(m.tool_call_id)),
        ("seq", json!(m.seq)),
        ("error", json!(m.error)),
    ] {
        let _ = n.attributes.insert(k.to_string(), v);
    }
    let _ = n
        .attributes
        .insert("meta".to_string(), m.meta.clone().unwrap_or(Value::Null));
    n
}

/// 消息正文——**流式追加的正是它**。
///
/// - `Text` / `Reasoning` / `UserPrompt`：纯文本正文；
/// - `Turn` / `ToolCall`（组合节点，本身无正文）：给一份稳定的 JSON 视图，
///   使前端与 LLM 在同一个地址上都能取到完整结构。
pub(crate) fn message_text(m: &cm::ChatMessage) -> String {
    match m.msg_type {
        Some(cm::MessageType::Turn) | Some(cm::MessageType::ToolCall) => {
            serde_json::to_string_pretty(m).unwrap_or_default()
        }
        _ => m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
    }
}

/// 转写按 `seq` 升序——`seq` 是唯一权威顺序锚点。
///
/// 稳定排序：缺 `seq` 的（本轮**在途**消息——存储尚未写入、因而还没分配 `seq`）
/// 排在最后并保持原有相对顺序，恰好落在「最新的消息在末尾」，不会被排到历史之前。
///
/// 快路径：存储本就按 `seq` 追加写入，`overlay_live` 也不改动既有顺序，因此
/// **已有序是常态**——此时排序是纯开销（`O(n log n)` 次比较 + 一次可能的整体重排），
/// 而转写的每次读取（`list` / `read` / `stat` 单条）都要过这里。
pub(crate) fn ordered(mut msgs: Vec<cm::ChatMessage>) -> Vec<cm::ChatMessage> {
    let key = |m: &cm::ChatMessage| m.seq.unwrap_or(i64::MAX);
    if !msgs.is_sorted_by_key(key) {
        msgs.sort_by_key(key);
    }
    msgs
}

/// 把本轮**在途**消息叠加到落库转写上。
///
/// 同 id 时在途版本胜出（它是更新的那一份），但 `seq` 例外——顺序锚点只由存储
/// 在写入时分配，在途副本没有，必须从落库版本继承。否则同一条消息会以
/// 「有 seq / 无 seq」两种形态被排到列表的两个位置。
pub(crate) fn overlay_live(
    stored: Vec<cm::ChatMessage>,
    live: Vec<cm::ChatMessage>,
) -> Vec<cm::ChatMessage> {
    let mut out = stored;
    for l in live {
        match out.iter_mut().find(|m| m.id == l.id) {
            Some(s) => {
                let seq = s.seq;
                *s = l;
                s.seq = seq;
            }
            None => out.push(l),
        }
    }
    out
}

/// 按 id 取单条消息
pub(crate) fn message_of<'a>(
    msgs: &'a [cm::ChatMessage],
    mid: &str,
) -> vdfs::VdfsResult<&'a cm::ChatMessage> {
    msgs.iter()
        .find(|m| m.id == mid)
        .ok_or_else(|| vdfs::VdfsError::not_found(format!("消息不存在：{mid}")))
}

/// 会话内容（转写全文 + 元数据）→ VDFS 文本内容。
///
/// ## `live` 不是可选装饰
///
/// 会话叶子（`<根>/session/<id>`）与转写列表（`<id>/message`）是**同一份数据的两个
/// 地址**，必须给出**同一份消息集合**。转写列表经 [`super::vdfs_provider`] 的
/// `transcript_of` 叠加了在途缓冲，因此叶子也必须叠加——否则「读叶子拿历史」与
/// 「按变更拼实时」两条路径会在流式期间分叉：叶子少掉**正在跑的那一轮**。
///
/// 这不是理论风险：前端 `loadMessages` 走的正是叶子（`fetchTranscript` →
/// `readVdfs(vdfsSessionAddr(id))`）。只读存储的话，Turn 运行中切走再切回就会看到
/// 「正在跑的消息凭空消失」，直到 `persist_messages` 在每轮结束时落库为止。
pub(crate) fn session_content(
    session: &super::super::types::Session,
    live: Vec<cm::ChatMessage>,
) -> vdfs::VdfsResult<vdfs::VdfsContent> {
    // 与 `transcript_of` 同一组合（`ordered(overlay_live(..))`），不另立口径：
    // 同 id 在途版本胜出（它更新），顺序锚点仍由存储分配的 `seq` 决定。
    let messages = ordered(overlay_live(session.messages.clone(), live));
    let payload = json!({
        "id": session.id,
        "title": session.display_title(),
        "metadata": session.metadata,
        "messages": messages,
        "updated_at": session.updated_at,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|e| vdfs::VdfsError::internal(format!("会话序列化失败：{e}")))?;
    Ok(vdfs::VdfsContent::text(text).with_mime("application/json"))
}

/// 具名新建时，**地址末段即会话 id**；写在挂载根（无名目标）返回 `None`。
///
/// 「**有名字**时 id 来自地址（使用方给），**没名字**时 id 由 provider 生成」是
/// VDFS 的**通用**规则，两处规范同义：`providers/vdfs_service/entry.rs::id_of`
/// 的注释，以及 [`vdfs::VdfsProvider::write`] 的「两种目标形态」表
/// （具名节点 + 不存在 ⇒ **就地创建**；只有目录自身才「名字由 provider 生成」）。
///
/// ## 为什么不剥 `.session` 后缀
///
/// 会话寻址里扩展名**从来不是**地址的一部分，也**从来不被剥除**：
/// `Session(id)` 直接把末段当 id 用（`parse_session_path` → `session_of`）。
/// 只在这里剥会造出「同一个 id 有两种写法、其中一种只在新建时成立」的怪状态，
/// 比不剥更糟。
pub(crate) fn session_id_from_new_path(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path).trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

#[cfg(test)]
#[path = "nodes.test.rs"]
mod tests;
