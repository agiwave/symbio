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
    Failed,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

impl ChatMessage {
    /// 把 `patch` 合并进 `self`（**增量语义**，与前端 `handleChatEvent` 对齐）。
    ///
    /// # 为什么必须合并而不是整条覆盖
    ///
    /// 调用方常常只构造"局部补丁"（例如 `ChatMessage { id, meta, ..Default::default() }`，
    /// 只为给一个 ToolCall 打 `recoverable` 标记）。若存储层用 `*existing = patch` 整条覆盖，
    /// 该消息的 `role` / `msg_type` / `content` / `name` / `timestamp` 会被一并抹成 `None`：
    ///   · 前端渲染丢失类型与内容（节点退化成空壳）；
    ///   · 下一轮 `flatten_chat_messages` 把它转成一条 `role` 缺失、`content` 缺失的
    ///     native message，请求体里出现 `"content": null`，Provider 侧 untagged enum
    ///     `MessageContent` 反序列化失败 → 整个会话被一条 400 卡死。
    /// 因此这里统一规定：`None` 表示"不修改该字段"，`Some` 才覆盖。
    ///
    /// # 合并规则
    ///
    /// - `content`：`Text` / `Reasoning` 走 **增量追加**（SSE delta 语义）；
    ///   `ToolCall` 与 `Parts` 走 **全量替换**（每帧都是完整参数/内容）。
    /// - `meta`：浅合并（同键以 patch 为准）。
    /// - 其余字段：`Some` 覆盖，`None` 保留原值。
    /// - `error`：显式区分"清空"——只有 patch 携带 `error` 字段时才生效，
    ///   由于 `Option<String>` 无法区分"不修改"和"清空"，约定 `Some(String::new())` 表示清空。
    pub fn apply_patch(&mut self, patch: &ChatMessage) {
        if let Some(role) = &patch.role {
            self.role = Some(role.clone());
        }
        if let Some(t) = &patch.msg_type {
            self.msg_type = Some(t.clone());
        }
        if let Some(n) = &patch.name {
            self.name = Some(n.clone());
        }
        if let Some(p) = &patch.parent_id {
            self.parent_id = Some(p.clone());
        }
        if let Some(s) = &patch.status {
            self.status = Some(s.clone());
        }
        if let Some(ts) = patch.timestamp {
            self.timestamp = Some(ts);
        }
        // seq 是顺序锚点，只能由存储层在写入时分配；补丁里不带就表示"保持不变"。
        if let Some(s) = patch.seq {
            self.seq = Some(s);
        }
        if let Some(rid) = &patch.response_id {
            self.response_id = Some(rid.clone());
        }
        if patch.prompt.is_some() {
            self.prompt = patch.prompt.clone();
        }
        match &patch.error {
            Some(e) => {
                // Some("") 语义为"清空错误原因"
                self.error = if e.is_empty() { None } else { Some(e.clone()) };
            }
            None => {}
        }

        if let Some(new_content) = &patch.content {
            match self.msg_type {
                Some(MessageType::ToolCall) => {
                    // 工具调用参数是每帧全量的 JSON
                    self.content = Some(new_content.clone());
                }
                Some(MessageType::Text) | Some(MessageType::Reasoning) => {
                    match (&mut self.content, new_content) {
                        (Some(existing), MessageContent::Text(new_text)) => {
                            match existing {
                                MessageContent::Text(buf) => buf.push_str(new_text),
                                other => *other = MessageContent::Text(new_text.clone()),
                            }
                        }
                        (None, MessageContent::Text(new_text)) => {
                            self.content = Some(MessageContent::Text(new_text.clone()));
                        }
                        _ => self.content = Some(new_content.clone()),
                    }
                }
                _ => self.content = Some(new_content.clone()),
            }
        }

        if let Some(new_meta) = &patch.meta {
            match &mut self.meta {
                Some(existing) => {
                    if let (Some(dst), Some(src)) = (existing.as_object_mut(), new_meta.as_object())
                    {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    } else {
                        self.meta = Some(new_meta.clone());
                    }
                }
                None => self.meta = Some(new_meta.clone()),
            }
        }
    }
}

/// 取一批消息中已有的最大 `seq`，作为后续分配的起点（Lamport 计数器的当前水位）。
pub fn max_seq(messages: &[ChatMessage]) -> i64 {
    messages.iter().filter_map(|m| m.seq).max().unwrap_or(0)
}

/// 按切片顺序为"尚未分配序号"的消息补发 `seq`，返回分配后的新水位。
///
/// # 语义（Lamport 计数器）
///
/// - 起点是 `base`（通常是当前会话已有的最大 seq），新值严格 `base+1, base+2, …`；
/// - 已带 `seq` 的消息**保持原值不动**（避免重排历史）；
/// - 由于起点取自"已有最大值"，跨调用、跨进程重启都不会回退或撞号。
///
/// `base = max_seq(existing)` 需在调用前算好：本批次内已带 seq 的消息不参与递增，
/// 但它们的 seq 可能大于 base，因此返回"实际达到的最大 seq"作为新水位。
pub fn assign_seq(messages: &mut [ChatMessage], base: i64) -> i64 {
    let mut cursor = base;
    for m in messages.iter_mut() {
        match m.seq {
            Some(existing) => {
                if existing > cursor {
                    cursor = existing;
                }
            }
            None => {
                cursor += 1;
                m.seq = Some(cursor);
            }
        }
    }
    cursor
}

/// 会话恢复动作。
///
/// 统一覆盖所有"非普通消息"的用户主动操作（与 `session_chat::Request.message` 互斥）：
/// - `RetryTurn`：LLM 失败重试（删除 Failed Turn 及其所有子节点，重新走 LLM 请求）
/// - `Retry`：工具执行失败后重试（用原 args）
/// - `Approve` / `Reject`：confirm 审批的批准/拒绝
/// - `Supply`：工具执行失败后补充参数重试（合并 args）
/// - `Answer`：ask_user 提问的答案回填
///
/// 恢复语义：删除旧消息 → 重新执行 → 生成新的响应子节点。
/// 对于工具调用场景，ToolCall 父节点 id 保持不变（稳定锚点），仅状态更新。
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
}

/// 会话恢复请求（与 `session_chat::Request.message` 互斥；存在时走 resume 分支）。
///
/// 由 session 插件透传到 `model_chat::Request.resume`，最终在
/// `model/chat_loop.rs:run_chat_loop` 的 turn 循环前由 `model/resume.rs:process_resume` 处理。
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
