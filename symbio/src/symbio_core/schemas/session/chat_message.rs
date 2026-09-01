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
            },
            MessageContent::Parts(_) => {
                let text = self.to_text();
                if text.len() > max_len {
                    let truncated: String = text.chars().take(max_len).collect();
                    *self = MessageContent::Text(truncated);
                }
            },
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
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
