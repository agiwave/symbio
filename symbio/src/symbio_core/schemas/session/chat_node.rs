use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Principal;

/// 消息角色
///
/// 请求 / 响应由角色区分（分型结构）：
/// - 顶层会话：`User`(请求) → `Turn`(`Assistant`，响应)
/// - 工具调用：`ToolCall`(`Assistant`，请求) → `Turn`(`Tool`，响应)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeRole {
    #[default]
    User,
    Assistant,
    Tool,
    System,
}

/// 节点**形状**——五值闭集（`body` / `notice` / `skeleton` / `interaction` / `typing`），
/// 是前端呈现的**唯一分派轴**（[ADR-070 §决策 3]，[T-155]）。这五值是**纯 UI 形状**概念
/// （展开叶 / 折叠叶 / 容器 / 交互 / 占位），不含任何业务。同 shape 内的业务变体
/// （run/tool/reasoning/compression…）走 `meta.style_tag`，**只在后端作业务判据**，
/// 前端从不读它——业务差异呈现靠 `title`/`icon`/`actions` 数据（[ADR-070 §决策 2]）。
///
/// 消费方按 [`ChatNode`] 上的**后端谓词**（`is_run` / `is_tool_call` / `is_reasoning` /
/// `is_text` / `is_compression` / `is_interaction`）判节点，而不是按变体名 match——
/// 变体名 + tag 两维合起来才是业务语义（这二维只活在后端）。
///
/// 旧六词（`text` / `reasoning` / `turn` / `tool_call` / `user_prompt` / `compression`）
/// **不在本枚举**：它们只存在于存量 VDFS 属性里，经 [`legacy_shape_to_style_tag`]
/// 在 [`message_of_node`] 读边界一次性折叠成新形态 + `style_tag` 注入。
///
/// 决策依据：[ADR-070 §决策 1 / 3]·父批：[T-155]。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeKind {
    /// 正文叶子：展开显示。业务呈现变体（思考 / 工具响应文本 …）**不在这一格**，
    /// 靠 `title` / `icon` 数据区分；前端只认「这是一个展开的正文叶子」。
    #[default]
    Body,
    /// 折叠叶子：**默认收起**的内容。`reasoning`（思考）与 `compression`（压缩）与
    /// 未来 `step_summary` 全在这一格——前端渲染它们**完全一样**（折叠叶 + `title` + `icon`），
    /// 不需要知道这坨叫思考还是压缩（业务区分只在后端 `style_tag` 注记 + 数据）。
    Notice,
    /// 组合骨架 / 容器：递归渲染 `children`。`run`（一整轮）、`tool`（一次工具调用）、
    /// `sub_agent` 全在这一格——前端一律当容器，不认 tool/turn（[ADR-070 §决策 1]）。
    Skeleton,
    /// 等待用户回应（`ask_user` 提问或工具执行前确认）
    Interaction,
    /// 进行中的占位（前端合成，见 [T-151]）
    Typing,
}

/// **旧 wire 词 → 目标形状 + style_tag**（[ADR-070 §决策 3 / 6]）。
///
/// 消费点是 `message_of_node`：VDFS `attributes.type` 存的是**当时的 wire 词**——历史数据里
/// 有 `"text"` / `"reasoning"` / `"turn"` / `"tool_call"` / `"user_prompt"` / `"compression"`
/// 六种旧值；本函数把它们折成新形态并**返回**应写入 `meta.style_tag` 的值（`None` = 不写）。
/// 新词（`"body"` / `"notice"` / `"skeleton"` / `"interaction"` / `"typing"`）也认。
///
/// 旧六值已从 `NodeKind` 删除，但存量数据的旧词**永久**需要本函数折叠——
/// 删的是枚举变体，不是已落盘的 wire 词。
pub fn legacy_shape_to_style_tag(word: &str) -> Option<(NodeKind, Option<&'static str>)> {
    Some(match word {
        // 新词——原样，无 style_tag 需要注入。
        "body" => (NodeKind::Body, None),
        "notice" => (NodeKind::Notice, None),
        "skeleton" => (NodeKind::Skeleton, None),
        "interaction" => (NodeKind::Interaction, None),
        "typing" => (NodeKind::Typing, None),
        // 旧词——折成新形态 + 对应 style_tag。
        "text" => (NodeKind::Body, None),
        // [ADR-070 §决策 3]：reasoning 归 `Notice`（折叠叶），与 compression 同格——
        // 前端渲染二者完全一样，style_tag 只是后端业务注记。
        "reasoning" => (NodeKind::Notice, Some(CHAT_STYLE_TAG_REASONING)),
        "turn" => (NodeKind::Skeleton, Some(CHAT_STYLE_TAG_RUN)),
        "tool_call" => (NodeKind::Skeleton, Some(CHAT_STYLE_TAG_TOOL)),
        "user_prompt" => (NodeKind::Interaction, None),
        "compression" => (NodeKind::Notice, Some(CHAT_STYLE_TAG_COMPRESSION)),
        _ => return None,
    })
}

/// **style_tag 词表**（[ADR-070 §决策 3]）——同一 shape 下的**后端业务注记**。
///
/// [T-155] 起它**不再是前端呈现分派轴**：前端只按 `NodeKind` 五值 + `title`/`icon`/`actions`
/// 数据渲染，从不 `switch(style_tag)`。本词表住在 `meta` 里，只服务后端业务判据
/// （`is_run` / `is_tool_call` / `is_reasoning` / `is_compression`）与 LLM 请求重建。
/// 后端认识业务天经地义；前端不认。加一个业务变体 = 加一行常量 + 后端填 `title`/`icon`，前端零改动。
pub const CHAT_STYLE_TAG_REASONING: &str = "reasoning";
/// `Skeleton` 的一种：一整轮助手响应（曾经的 `Turn`）。
pub const CHAT_STYLE_TAG_RUN: &str = "run";
/// `Skeleton` 的一种：一次工具调用（曾经的 `ToolCall`）。
pub const CHAT_STYLE_TAG_TOOL: &str = "tool";
/// `Notice` 的一种：上下文压缩记录（曾经的 `Compression`）。
pub const CHAT_STYLE_TAG_COMPRESSION: &str = "compression";

/// 消息状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeStatus {
    #[default]
    Pending,
    Streaming,
    WaitingUserAction,
    Completed,
    /// 用户主动终止。
    ///
    /// 它既不是 `Failed`（没有出错，是用户按了停止）也不是 `Completed`（没有跑完，
    /// 内容是半截的）。此前中止只能二选一：冒泡路径标 `Failed`（给一个 ⚠ 错误角标，
    /// 把用户自己的操作渲染成故障），其余路径标 `Completed`（说谎——那一轮根本没跑完）。
    ///
    /// 语义上它**可重试**：`RetrySubtree` 本就不看状态（只校验节点是骨架），
    /// 缺的只是前端一个「这个终态可以重试」的判据。
    Aborted,
    Failed,
    /// 节点已被删除（**终态，不可重试**）。
    ///
    /// 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成
    /// 同走一帧。落在此状态的帧在接收端**就地移除**该节点（从本地视图消失），
    /// 不产生任何「清空重读」。
    Removed,
}

impl NodeStatus {
    /// 状态词——**与 `serde` 的序列化名逐字一致**（由单测锁死）。
    ///
    /// ## 为什么需要它
    ///
    /// VDFS 节点投影（`plugins/session/plugin/nodes.rs::message_status`）要把状态
    /// 写进 `VdfsNode.status`，而前端按同一套词读回。若那里手写字符串字面量，
    /// 枚举改名时就会出现「存储写 `streaming`、节点报 `streaming` 之外的词」
    /// 这种**编译期看不见**的分叉。
    ///
    /// 于是状态词只有一份定义（本函数），投影与序列化都从它派生。
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeStatus::Pending => "pending",
            NodeStatus::Streaming => "streaming",
            NodeStatus::WaitingUserAction => "waiting_user_action",
            NodeStatus::Completed => "completed",
            NodeStatus::Aborted => "aborted",
            NodeStatus::Failed => "failed",
            NodeStatus::Removed => "removed",
        }
    }

    /// [`Self::as_str`] 的逆：状态词 → 枚举。未知词返回 `None`。
    ///
    /// 存在的理由与 `as_str` 是同一条的另一半：VDFS 变更**不带载荷**（ADR-025），
    /// 消费端拿到 `<sid>/message/<mid>` 的 `updated` 后只能回读**节点**
    /// （`stat`），而节点上承载状态的是那个**词**。把它解析回枚举若在每个消费端
    /// 各手写一次 match，词表就又有了第二份定义。
    pub fn of(word: &str) -> Option<Self> {
        match word {
            "pending" => Some(NodeStatus::Pending),
            "streaming" => Some(NodeStatus::Streaming),
            "waiting_user_action" => Some(NodeStatus::WaitingUserAction),
            "completed" => Some(NodeStatus::Completed),
            "aborted" => Some(NodeStatus::Aborted),
            "failed" => Some(NodeStatus::Failed),
            "removed" => Some(NodeStatus::Removed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn is_empty(&self) -> bool {
        match self {
            MessageContent::Text(s) => s.is_empty(),
            MessageContent::Parts(p) => p.is_empty(),
        }
    }

    pub fn to_text(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    pub fn len(&self) -> usize {
        self.to_text().len()
    }

    pub fn truncate(&mut self, max_len: usize) {
        match self {
            MessageContent::Text(s) => {
                if s.len() > max_len {
                    *s = s.chars().take(max_len).collect();
                }
            }
            MessageContent::Parts(_) => {
                let text = self.to_text();
                if text.len() > max_len {
                    let truncated: String = text.chars().take(max_len).collect();
                    *self = MessageContent::Text(truncated);
                }
            }
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        MessageContent::Text(String::new())
    }
}

/// 聊天消息定义 (所有字段增量可选，通过 id 归并)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatNode {
    pub id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    /// **存量只读格**（[T-147d-iii]）：`role` 已退出 wire（`skip_serializing` ⇒
    /// 写方不再发、读侧折叠按 `principal.kind` 走），这一格只为**存量数据**保留：
    /// 历史 `messages.json` / 容器正文里带 `"role"` 的节点反序列化进来后，
    /// [`Self::node_role`] 仍按「格先 kind 后」折叠——与前端 `messageRoleOf` 同一判据。
    ///
    /// 为什么不是私有字段：`ChatNode { .., ..Default::default() }` 的**函数式更新
    /// 语法要求每个字段可见**（E0451），私有化会把全仓数百个不碰 `role` 的构造点
    /// 一起炸掉——机制的爆炸半径大于收益。约束改由两条腿承担：读侧一律走
    /// [`Self::node_role`] / [`Self::legacy_role`]（分派判据只此一处），写侧新增
    /// 直写由 `scripts/grep-audit.mjs`（S-014）棘轮钉住（只许降）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<NodeRole>,

    /// **这条发言是谁说的**（[plan/11 批 1](../../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
    /// ② 的跨栈身份字段）：主体地址（`{kind, id, title?}`，[ADR-069 §决策 1]）。
    ///
    /// `None` = 尚未标身份（历史消息、未接入的写方）——可见域判据把它当**可见**：
    /// 缺身份是「还没接线」，不是「不让人看」。反过来把 `None` 判成不可见会让
    /// 所有存量消息从视图里消失，那是拿接线覆盖旧行为（S08 §4 平凡值要求逐字一致）。
    ///
    /// **读侧兼容**（[T-147b]，[ADR-069 §决策 6]）：wire 上历史 `messages.json` 里的
    /// 裸字符串（`"agent:main"` / `"user"` / `"session/main"`）由 [`Principal::deserialize`]
    /// 一次性映射成结构体；写侧一律构造 `Principal`，序列化只出对象形态。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<Principal>,

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<NodeKind>,

    /// [T-147d-ii]：顶层 `name` 字段**已退役**——工具名是结构细节，住
    /// `meta.tool_name`（读写唯一入口 [`ChatNode::tool_name`] /
    /// [`ChatNode::with_tool_name`]）。存量两条路径都在读边界折进来：
    /// VDFS 属性 `tool_name`（[`message_of_node`]）与容器节点正文 JSON 的
    /// `name` 键（serde 忽略未知键，靠属性那条兜住）。

    #[serde(skip_serializing)]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,

    /// 流式**增量**：本帧新到达的那一段正文，追加到接收端该节点正文尾部。
    ///
    /// ## 与 `content` 互斥，语义由字段本身给出
    ///
    /// 一帧里两者**绝不同时出现**——同帧携带即协议违例，唯一写入点
    /// （`plugins/session/transcript.rs::Transcript::apply`）报错丢弃。
    /// 消费端因此永远不必从帧的形状里推断「该拼接还是该替换」：
    ///
    /// | 字段 | 语义 | 用在哪 |
    /// |---|---|---|
    /// | `delta` | 追加到正文尾部（O(delta) 窄帧） | 流式热路径：正文 / 推理 / 工具参数逐片 |
    /// | `content` | **整条替换**该节点正文（幂等） | 一次性节点的完整体、存储回执的权威副本、编辑后的新正文 |
    ///
    /// ## 为什么不落存储
    ///
    /// 增量是**不完整的片段**，不是消息的形态：它只在出方向的帧上存在，
    /// 存储里只有 `content`（图内累积的正文）。改名的同一份结构因此承载两件事：
    /// - 帧方向：`delta` 是「这一段」；
    /// - 存储方向：`content` 是「全部」。
    ///
    /// 「delta 永不落盘」不是发射方的自律，而是持久层的不变量——
    /// `chat_session::write::ensure_durable_states` 在写入点直接拒绝携带 `delta` 的消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatus>,

    /// 失败原因（面向用户的可读短消息）。仅当 `status == Failed` 时存在。
    /// 用于把"会话异常中断/失败"的终态持久化到历史，使切换会话后仍能看到上次错误。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// 呈现标题（[ADR-070 §决策 2 / 3]，[T-155]）：后端在构造节点时填的**面向用户显示文本**
    /// （思考="思考过程" / 压缩="上下文压缩" / 工具=工具名 / 轮=智能体名）。
    /// 前端**照数据显示**，不再按业务类型查表生成——这是"前端不懂业务"的落点之一。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// 呈现徽标标识（[T-155]）：后端填的**图标词**（不是颜色、不是业务词）；前端把标识
    /// 映射到 UI 资产表（class→svg 那种映射，不是 `switch(type)`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// 本节点**可发起的恢复动作**（[ADR-070 §决策 3]，[T-155]）：后端算好「这个节点现在
    /// 能重试 / 审批 / 补参 / 回答 / 重跑压缩」中的哪几种，随节点下发。前端只做
    /// 「列表里有 `retry` 就画重试按钮」的**数据渲染**，不再按业务类型（tool/compression/run）
    /// 反推该发哪种动作——那是前端最后一处业务耦合，本字段把它搬回后端。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ResumeAction>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,

    /// 会话内**单调自增序号**：消息先后顺序的权威锚点。
    ///
    /// ## 为什么不用 `timestamp` 排序
    ///
    /// `timestamp` 是"业务时刻"，不是"顺序"。拿它当排序键会有三个硬伤：
    /// - **并列**：一轮里的 Turn + Reasoning + Text + 多个 ToolCall 常常在同一毫秒内
    ///   批量落库，`timestamp` 完全相同，排序只能退化成"看数组当前顺序"，
    ///   而数组顺序本身又可能被上一次错误排序打乱，形成自我强化的错乱；
    /// - **逆序**：时钟回拨、多进程/多会话写入会让"晚产生"的消息拿到更小的时间戳；
    /// - **缺失**：流式补丁路径不写 `timestamp`，需要哨兵值（如 `i64::MAX`）兜底，
    ///   本质是在用魔法值掩盖语义缺失。
    ///
    /// `seq` 由会话存储在**写入时**分配（Lamport 计数器：从已有最大 seq 续上），
    /// 单调递增且永不并列，因此能无损恢复插入顺序，且跨调用、跨进程重启都不回退。
    ///
    /// **字段在本层，分配策略不在**：本结构是**跨栈 schema**（前端逐字段镜像），
    /// 故 `seq` 必须在这里；而「怎么分配、什么时候补号、整表重写时谁让位」是
    /// 会话存储的实现策略，住在 session 插件的 `chat_session.rs`
    /// （`max_seq` / `assign_seq`，与两条写入路径 `append_messages` /
    /// `replace_messages` 同处一文件）。core 只声明**存在一个顺序锚点**，
    /// 不规定它怎么被算出来。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,

    /// ToolCall 节点的 **wire id**（LLM provider 返回的 `tool_call_id`）。
    ///
    /// ## 为什么节点 id 不能直接用它
    ///
    /// 许多 OpenAI 兼容网关**跨轮复用** `call_0` / `call_xxx` 这类短 id。而消息
    /// id 在三处被当作**唯一键**：VDFS 地址（`<sid>/message/<mid>`）、前端 store
    /// （按 id 的 map）、消费循环的在途合并。wire id 直接当节点 id 会让下一轮
    /// 的同 id 工具调用更新到上一轮的老节点（后端 `Vec` 存储不撞，刷新后"自愈"
    /// ——正是"后端正常、前端显示混乱"的根源）。
    ///
    /// 因此 ToolCall 节点 id 在**诞生时**分配（会话内唯一），provider 的原始 id
    /// 存放在本字段，仅在构建 LLM 请求包时使用（部分协议要求回传原值，如
    /// OpenAI Responses 的 `call_id` 链）。历史数据无此字段：请求构建回退节点 id。
    ///
    /// ## [T-147d-i]：**wire 上不存在，只在后端内存里活着**
    ///
    /// `#[serde(skip)]` 让它**不进出** `ChatNode` 的序列化形状——前端从不消费它
    /// （过去只是 copy-through 到 store），LLM 请求构造侧仍走 Rust 结构体字段读取。
    /// VDFS 属性投影（`plugins/session/plugin/nodes.rs`）与 WAL 事件载荷
    /// （`turn_runner.rs::artifact.added`）**独立于本字段的 serde 属性**，仍把
    /// wire id 写进 `attributes.tool_call_id` / 事件 `tool_call_id` 键——那是后端
    /// 到后端的存储形状，不是前端 wire。[`message_of_node`] 逆投影照原路读回。
    #[serde(skip)]
    pub tool_call_id: Option<String>,
}

/// 会话恢复动作。
///
/// 统一覆盖所有"非普通消息"的用户主动操作（与 `session_chat::Request.message` 互斥）：
/// - `RetrySubtree`：LLM 失败重试（删除靶子节点及其所有子孙节点，重新走 LLM 请求）
/// - `Retry`：工具执行失败后重试（用原 args）
/// - `Approve` / `Reject`：confirm 审批的批准/拒绝
/// - `Supply`：工具执行失败后补充参数重试（合并 args）
/// - `Answer`：ask_user 提问的答案回填
/// - `RetryCompaction`：压缩失败重试（删除 Failed 压缩节点，重新执行一次压缩）
///
/// 恢复语义：删除旧消息 → 重新执行 → 生成新的响应子节点。
/// 对于工具调用场景，ToolCall 父节点 id 保持不变（稳定锚点），仅状态更新。
///
/// 线格式为 `snake_case`（见 `#[serde(rename_all)]`），与前端
/// `composables/useChatConnection.ts::ResumePayload.action` 的字面量逐字相等——
/// 这条跨栈契约由 `resume_action_wire_words_are_snake_case` 钉住。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ResumeAction {
    /// LLM 失败重试：删除靶子节点及其所有子孙节点，重新走 LLM 请求。
    ///
    /// [ADR-070 §决策 1]：曾经的 `RetryTurn` 语义绑 `Turn` 变体——靶子只允许是 Turn，
    /// 五值收敛（[T-147c-ii]）后 `Turn` 消失于 wire，本动词的语义随之泛化为「删除整棵
    /// 子树并重跑」，靶子是**任一骨架节点**（曾经的 Turn 或未来的 Skeleton+style_tag=run）。
    RetrySubtree,
    /// 工具失败重试：删除 Failed 工具结果子节点，重新执行工具（用原 args）
    Retry,
    /// 工具审批通过：删除 user_prompt 子节点，重新执行工具（带 approved=true）
    Approve,
    /// 工具审批拒绝：删除 user_prompt 子节点，生成拒绝结果子节点
    Reject,
    /// 工具补充参数：删除 Failed 工具结果子节点，合并参数后重新执行工具
    Supply,
    /// 提问回答：删除 user_prompt 子节点，生成答案结果子节点
    Answer,
    /// 压缩失败重试：删除 Failed 压缩节点，重新执行一次上下文压缩。
    ///
    /// `target_id` 指向**压缩节点**（`msg_type = Notice` 且 `is_compression()`、`status = Failed`）。
    /// 语义上属"删除-重建"，与其余恢复动作同款；但**不改动历史**——压缩失败
    /// 本身从不丢消息（见 `CompressionFailure` 的说明），重试只是再试一次
    /// LLM 摘要，成功后由快照接替被压缩的那段。
    RetryCompaction,
}

/// 会话恢复请求（与 `session_chat::Request.message` 互斥；存在时走 resume 分支）。
///
/// 由 session 编排层随请求透传，最终在 session 插件会话循环
/// （`session/chat_loop.rs:run_chat_loop`）的 turn 循环前由 `session/resume.rs:process_resume` 处理。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResumeRequest {
    /// 目标消息 ID（Failed Turn 或 ToolCall，稳定标识，覆盖式更新锚点）
    pub target_id: String,
    pub action: ResumeAction,
    /// supply 时的补充参数（与原 args 浅合并）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// reject 时的拒绝原因
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// answer（ask_user）时的答案对象
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
}

// ==================== 转写地址与节点投影 ====================
//
// 下面两项从 `plugins/session` 上移到 core：它们描述的是**跨插件的契约**，
// 不是会话插件的内部实现。上移之前，`plugins/agent` 为了构造子会话转播桥，
// 不得不 `use crate::plugins::session::plugin::{…}` —— 那是全仓**唯一**一处
// 真跨插件引用，违反 `plugins/mod.rs` 声明的「插件独立原则」。
//
// 判据：一个符号该不该在 core，看**消费方是谁**——只要有一个消费方不是它所在
// 的那个插件，它就不是那个插件的私事（与 ADR-023「准入判据 = 依赖方数量」同款）。

/// 会话内部：转写列表的**路径段**（ASCII，进地址）。
///
/// 转写是**列表**：`<根>/session/<id>/message` 的每一项是一条消息，顺序由
/// `seq`（唯一权威顺序锚点）决定。这个地址只服务**读面**（一次 `read` 拿整份
/// 历史）与**写面**（`vdfs/action` 的截断 / 清空）。
///
/// **实时面不在这个目录上**：一条消息的流式 = 它**自己那个地址**
/// （`<id>/message/<mid>`）上的 `delta` 增量，目录只承载列表。把实时面挂在目录上
/// 会让 `path` 的含义随帧类型漂移，且无法推广到第二类集合——理由见
/// ADR-025 追记。
pub const SEG_MESSAGES: &str = "message";

/// [`ChatNode`] 的**逆投影**：VDFS 节点 + 正文 ⇒ 一条消息。
///
/// ## 为什么需要它
///
/// VDFS 变更**只在热路径上带载荷**：消息帧的 `data` 就是那条 `ChatNode`
/// （`delta` / `content` / `status`），而资源信号是**无载荷**的。因此「拿到一条
/// 无载荷变更、要还原成消息」的消费端（如 agent 转播桥）只能 `stat` + `read`。
///
/// ## 为什么在 core
///
/// 它必须与「拼」（`plugins/session/plugin/nodes.rs::message_node`）**成对演进**：
/// `attributes` 增字段时不可能只改一边。而消费方（agent 的转播桥）不是 session
/// 插件——契约住 core，两侧都依赖它，就不会有一侧从对方内部「借」实现。
///
/// `None` = 节点状态词不在 [`NodeStatus`] 的词表里（正常不该发生；
/// 发生即两侧已分叉，宁可丢这一条也不要造出一个状态错误的消息）。
pub fn message_of_node(node: &crate::symbio_core::VdfsNode, text: String) -> Option<ChatNode> {
    /// attributes 里的值都是 `json!(..)` 塞进去的，原样反序列化即可回读类型。
    fn attr<T: serde::de::DeserializeOwned>(
        m: &serde_json::Map<String, Value>,
        k: &str,
    ) -> Option<T> {
        serde_json::from_value(m.get(k).cloned()?).ok()
    }
    // `attributes.type` 按**词**读再走 [`legacy_shape_to_style_tag`] 折叠，而不是
    // serde 直进 `NodeKind`：存量数据里存的是当时的 wire 词（旧六值已不在枚举里，
    // serde 会静默失败成 `None`）。折叠产出的 style_tag 只在 meta **还没有**该键时
    // 注入——新写方带着完整形态进来时不覆盖。
    let (msg_type, legacy_tag) = attr::<String>(&node.attributes, "type")
        .as_deref()
        .and_then(legacy_shape_to_style_tag)
        .map(|(kind, tag)| (Some(kind), tag))
        .unwrap_or((None, None));
    let mut meta = node
        .attributes
        .get("meta")
        .cloned()
        .filter(|v| !v.is_null());
    // 存量属性里的旧结构键**非覆盖**折进 meta（与 style_tag 同一条纪律：新写方
    // 带着完整 meta 进来时不覆盖）。[T-147d-ii]：顶层 `name` 退役后，属性
    // `tool_name` 的唯一归宿是 `meta.tool_name`。
    for (key, value) in [
        ("style_tag", legacy_tag.map(Value::from)),
        (
            "tool_name",
            attr::<String>(&node.attributes, "tool_name").map(Value::from),
        ),
    ] {
        let Some(v) = value else { continue };
        if meta
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .is_some()
        {
            continue;
        }
        let slot = meta.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = slot.as_object_mut() {
            obj.insert(key.to_string(), v);
        }
    }
    Some(ChatNode {
        id: node.name.clone(),
        role: attr(&node.attributes, "role"),
        msg_type,
        parent_id: attr(&node.attributes, "parent_id"),
        tool_call_id: attr(&node.attributes, "tool_call_id"),
        seq: attr(&node.attributes, "seq"),
        error: attr(&node.attributes, "error"),
        principal: attr(&node.attributes, "principal"),
        // `message_node` 把「没有 meta」写成 `null`；这里还原成"没有"
        meta,
        status: Some(NodeStatus::of(&node.status)?),
        content: Some(MessageContent::Text(text)),
        ..Default::default()
    })
}

impl ChatNode {
    /// 读 `meta.tool_name`——工具名的**唯一**读写入口（[T-147d-ii]，替代已退役的
    /// 顶层 `name` 字段）。值是 LLM 可见工具名（可能是限定形态 `local/todo_write`，
    /// 也可能裸名，取决于产地）。
    pub fn tool_name(&self) -> Option<&str> {
        self.meta
            .as_ref()
            .and_then(|m| m.get("tool_name"))
            .and_then(Value::as_str)
    }

    /// 写 `meta.tool_name`（链式构造 helper，[T-147d-ii] 构造侧）。`None` 不写
    /// （缺席=没有，不往 meta 里塞 null）。
    pub fn with_tool_name(mut self, name: Option<String>) -> Self {
        let Some(name) = name else { return self };
        let meta = self
            .meta
            .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("tool_name".to_string(), Value::String(name));
        }
        self
    }

    /// 归属折叠判据（[T-147d-iii]）：`role` 的**唯一读入口**。
    ///
    /// 共存阶段的优先级是「残留格优先、`principal.kind` 兜底」：今天仍有构造点只写
    /// `role` 不写 `principal`（子会话转播改写 `role=Tool` 这类），kind 折叠一旦
    /// **优先**就会静默改变它们的读侧分派——所以这里按「写过的不动，没写的从主体折」。
    /// 字段退役（d-iii 末档）后本函数收敛成纯 `principal.kind` 折叠：
    /// Person→User、Agent→Assistant、Tool→Tool、System→System（两域一一对应）。
    pub fn node_role(&self) -> Option<NodeRole> {
        use crate::symbio_core::principal::PrincipalKind;
        if self.role.is_some() {
            return self.role.clone();
        }
        Some(match self.principal.as_ref()?.kind {
            PrincipalKind::Person => NodeRole::User,
            PrincipalKind::Agent => NodeRole::Assistant,
            PrincipalKind::Tool => NodeRole::Tool,
            PrincipalKind::System => NodeRole::System,
        })
    }

    /// **过渡入口**（[T-147d-iii]）：写存量只读格 `role`。
    ///
    /// 语义上等价于直写字段，存在的理由是**可搜寻性**：写存量格的调用点全部
    /// 冠以 `with_role` / `set_legacy_role`，下一档清零时一处 grep 就是完整清单
    /// （直写 `node.role = ...` 与字段初始化会淹没在同名读写里）。只给
    /// 「身份还没迁到 `principal` 的现存写方」用；每个调用点都是待清零的债务，
    /// 新增调用需要理由（棘轮：`scripts/grep-audit.mjs` S-014）。
    pub fn with_role(mut self, role: NodeRole) -> Self {
        self.role = Some(role);
        self
    }

    /// [`Self::with_role`] 的 `&mut` 形态（同批过渡入口）：就地改写存量格，
    /// 供补丁应用 / 帧合并这类拿不到所有权的调用点用。
    pub fn set_legacy_role(&mut self, role: NodeRole) {
        self.role = Some(role);
    }

    /// 存量只读格的读法（仅 `attributed` 与 attrs 逆投影这类**读边界**用；
    /// 一般分派一律走 [`Self::node_role`]）。
    pub fn legacy_role(&self) -> Option<NodeRole> {
        self.role.clone()
    }

    /// 读 `meta.style_tag`（wire 层的呈现变体词，[ADR-070 §决策 3]）。
    pub fn style_tag(&self) -> Option<&str> {
        self.meta
            .as_ref()
            .and_then(|m| m.get("style_tag"))
            .and_then(Value::as_str)
    }

    /// 写入 `meta.style_tag`（链式构造 helper，[T-147c-ii] 构造侧）。
    ///
    /// `meta` 已有对象则并入该对象；`meta` 是非对象 JSON（不该发生）时原样保留、不注入。
    pub fn with_style_tag(mut self, tag: &'static str) -> Self {
        let meta = self
            .meta
            .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("style_tag".to_string(), Value::String(tag.to_string()));
        }
        self
    }

    /// 本节点是不是「一整轮助手响应的骨架」（`Skeleton` + tag `run`）。
    pub fn is_run(&self) -> bool {
        self.msg_type == Some(NodeKind::Skeleton) && self.style_tag() == Some(CHAT_STYLE_TAG_RUN)
    }

    /// 本节点是不是「一次工具调用骨架」（`Skeleton` + tag `tool`）。
    pub fn is_tool_call(&self) -> bool {
        self.msg_type == Some(NodeKind::Skeleton) && self.style_tag() == Some(CHAT_STYLE_TAG_TOOL)
    }

    /// 本节点是不是「推理正文」（`Notice` + tag `reasoning`）。
    ///
    /// [T-155]：reasoning 从 `Body + tag` 归位到 `Notice`（折叠叶，[PL-013 §3]）——
    /// 与 compression 同格，靠 `style_tag` 在后端区分。
    pub fn is_reasoning(&self) -> bool {
        self.msg_type == Some(NodeKind::Notice) && self.style_tag() == Some(CHAT_STYLE_TAG_REASONING)
    }

    /// 本节点是不是「普通正文」（`Body`）。
    ///
    /// [T-155]：reasoning 已离开 `Body`，故 `Body` 无需再排除 reasoning tag。
    pub fn is_text(&self) -> bool {
        self.msg_type == Some(NodeKind::Body)
    }

    /// 本节点是不是「压缩记录」（`Notice` 且**不是** reasoning）。
    ///
    /// [T-155]：`Notice` 现在同时承载 reasoning 与 compression，裸判 `msg_type==Notice`
    /// 会把思考也算成压缩——压缩判据必须显式排除 reasoning。存量无 tag 的裸 `Notice`
    /// 仍按压缩兜底（历史上 compression 是唯一 Notice 变体）。
    pub fn is_compression(&self) -> bool {
        self.msg_type == Some(NodeKind::Notice) && self.style_tag() != Some(CHAT_STYLE_TAG_REASONING)
    }

    /// 本节点是不是「等待用户回应的交互」（`Interaction`）。
    pub fn is_interaction(&self) -> bool {
        self.msg_type == Some(NodeKind::Interaction)
    }
}

#[cfg(test)]
#[path = "chat_node.test.rs"]
mod tests;
