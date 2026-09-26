use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 消息角色
///
/// 请求 / 响应由角色区分（分型结构）：
/// - 顶层会话：`User`(请求) → `Turn`(`Assistant`，响应)
/// - 工具调用：`ToolCall`(`Assistant`，请求) → `Turn`(`Tool`，响应)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageRole {
    #[default]
    User,
    Assistant,
    Tool,
    System,
}

/// 消息类型
///
/// `Turn` 与 `ToolCall` 是组合节点（不含实际内容，仅分组子节点）；
/// `Text` / `Reasoning` 是内容节点（携带可显示内容）。
/// 工具调用是分型结构：`ToolCall`(请求) 的子节点包含一个 `Turn`(`Tool`，响应)，
/// 该响应 `Turn` 与顶层助手响应 `Turn` 结构完全一致（可再嵌套 `ToolCall`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageType {
    #[default]
    Text,
    Reasoning,
    /// 组合节点：一轮响应（顶层为 `Assistant`，工具响应为 `Tool`），仅分组子节点
    Turn,
    /// 组合节点：一次工具调用（请求），携带 `name`，其子节点含请求 `Text` 与响应 `Text/Turn`
    ToolCall,
    /// 待用户响应节点：承载 `ask_user` 提问或工具执行前确认（confirm）。
    /// 状态为 `WaitingUserAction` 时表示等待用户输入，答案以一条普通 `user`
    /// 消息（`meta.responds_to` 指向本节点 id）回填，回填后本节点标记 `Completed`。
    /// 结构化载荷存放于 `meta.prompt`，详见会话激活/恢复状态机设计文档。
    UserPrompt,
    /// 上下文压缩节点：**不是对话内容**，是系统对历史的一次整理动作。
    ///
    /// 以消息节点而非「会话级横幅」呈现，是因为压缩本就是会话里发生的一步：
    /// 它有自己的位置（当前时刻）、自己的状态（进行中 / 已完成），被压掉的历史
    /// 就发生在它之前。横幅没有位置概念——用户滚到消息流中间时看不见它，
    /// 事后也无法回溯「上次压缩发生在哪里、压掉了多少」。
    Compression,
}

/// 消息状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MessageStatus {
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
    /// 语义上它**可重试**：`RetryTurn` 本就不看状态（只校验节点是 Turn），
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

impl MessageStatus {
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
            MessageStatus::Pending => "pending",
            MessageStatus::Streaming => "streaming",
            MessageStatus::WaitingUserAction => "waiting_user_action",
            MessageStatus::Completed => "completed",
            MessageStatus::Aborted => "aborted",
            MessageStatus::Failed => "failed",
            MessageStatus::Removed => "removed",
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
            "pending" => Some(MessageStatus::Pending),
            "streaming" => Some(MessageStatus::Streaming),
            "waiting_user_action" => Some(MessageStatus::WaitingUserAction),
            "completed" => Some(MessageStatus::Completed),
            "aborted" => Some(MessageStatus::Aborted),
            "failed" => Some(MessageStatus::Failed),
            "removed" => Some(MessageStatus::Removed),
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
pub struct ChatMessage {
    pub id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MessageRole>,

    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<MessageType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

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
    /// `chat_session::ensure_durable_states` 在写入点直接拒绝携带 `delta` 的消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,

    /// 失败原因（面向用户的可读短消息）。仅当 `status == Failed` 时存在。
    /// 用于把"会话异常中断/失败"的终态持久化到历史，使切换会话后仍能看到上次错误。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// 会话恢复动作。
///
/// 统一覆盖所有"非普通消息"的用户主动操作（与 `session_chat::Request.message` 互斥）：
/// - `RetryTurn`：LLM 失败重试（删除 Failed Turn 及其所有子节点，重新走 LLM 请求）
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
    /// LLM 失败重试：删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    RetryTurn,
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
    /// `target_id` 指向**压缩节点**（`msg_type = Compression`、`status = Failed`）。
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
/// `docs/DECISIONS.md` 的 ADR-025 追记。
pub const SEG_MESSAGES: &str = "message";

/// [`ChatMessage`] 的**逆投影**：VDFS 节点 + 正文 ⇒ 一条消息。
///
/// ## 为什么需要它
///
/// VDFS 变更**只在热路径上带载荷**：消息帧的 `data` 就是那条 `ChatMessage`
/// （`delta` / `content` / `status`），而资源信号是**无载荷**的。因此「拿到一条
/// 无载荷变更、要还原成消息」的消费端（如 agent 转播桥）只能 `stat` + `read`。
///
/// ## 为什么在 core
///
/// 它必须与「拼」（`plugins/session/plugin/nodes.rs::message_node`）**成对演进**：
/// `attributes` 增字段时不可能只改一边。而消费方（agent 的转播桥）不是 session
/// 插件——契约住 core，两侧都依赖它，就不会有一侧从对方内部「借」实现。
///
/// `None` = 节点状态词不在 [`MessageStatus`] 的词表里（正常不该发生；
/// 发生即两侧已分叉，宁可丢这一条也不要造出一个状态错误的消息）。
pub fn message_of_node(node: &crate::symbio_core::VdfsNode, text: String) -> Option<ChatMessage> {
    /// attributes 里的值都是 `json!(..)` 塞进去的，原样反序列化即可回读类型。
    fn attr<T: serde::de::DeserializeOwned>(
        m: &serde_json::Map<String, Value>,
        k: &str,
    ) -> Option<T> {
        serde_json::from_value(m.get(k).cloned()?).ok()
    }
    Some(ChatMessage {
        id: node.name.clone(),
        role: attr(&node.attributes, "role"),
        msg_type: attr(&node.attributes, "type"),
        name: attr(&node.attributes, "tool_name"),
        parent_id: attr(&node.attributes, "parent_id"),
        tool_call_id: attr(&node.attributes, "tool_call_id"),
        seq: attr(&node.attributes, "seq"),
        error: attr(&node.attributes, "error"),
        // `message_node` 把「没有 meta」写成 `null`；这里还原成"没有"
        meta: node
            .attributes
            .get("meta")
            .cloned()
            .filter(|v| !v.is_null()),
        status: Some(MessageStatus::of(&node.status)?),
        content: Some(MessageContent::Text(text)),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "chat_message.test.rs"]
mod tests;
