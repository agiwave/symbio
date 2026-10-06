//! `transcript` 投影——从事件网格读出**对话转写**（v2 会话运行时的历史侧）。
//!
//! [`crate::symbio_core::actors::Reasoner`] 组 prompt 的输入：多轮对话时模型
//! 需要看到自己说过什么（否则第二轮起失忆）。转写是**纯函数读视图**——
//! 历史来自同一份事实源，不另存副本（ADR-044：实测与判据同源的同族纪律）。
//!
//! 口径（**四格 → 三角色**）：
//! - `user.message`（`turn × opened`）→ `user` 行（载荷 `text`）；
//! - `chat.assistant.final`（`turn × closed`）→ `assistant` 行（载荷 `text`）；
//! - `chat.assistant.fallback`（`turn × closed`）→ `assistant` 行（载荷 `why`）——
//!   兜底话术是用户实际看到的回复，模型必须知道它「说」过这个；
//! - `artifact.added`（`artifact × asserted`）→ `tool` 行（载荷 `tool` + `text`）。
//!
//! ## 工具结果为什么也在转写里（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md)）
//!
//! 工具轮**当轮**的交换由运行器就地追加（`render_tool_exchange`：网格里还没有
//! 那部分历史，见 `actors::Reasoner::render_prompt` 的文档）；而**跨轮**的工具结果
//! 若不进投影，第二轮起的 prompt 就只剩「用户问过 X、助手答过 Y」——模型看不见
//! 自己调用过什么、拿到了什么，会**重复调用同一个工具**。所以 `artifact.added`
//! 进投影：多轮 prompt 因此能重建**含工具**的对话。
//!
//! 工具行渲染成 `工具结果(<tool>): <text>`——与轮内交换（`render_tool_exchange`）
//! **同一形态**，于是「历史里的工具轮」与「本轮的工具轮」在 prompt 里长得一样。

use super::super::event::{
    Entity, Event, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};

/// 转写条目：一句话及其角色。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// `"user"` / `"assistant"` / `"tool"`。
    pub role: String,
    pub text: String,
    /// 工具结果行的**工具名**（`role == "tool"` 时才有；其余角色为 `None`）。
    ///
    /// 单独成字段而不是拼进 `text`：工具名是**事实**（载荷里就有），渲染
    /// （`工具结果(<tool>): ` 前缀）是消费方的事。`skip_serializing_if` 让
    /// 非工具行的线格式保持 `{role, text}` 不变——新增角色不改旧角色的形状。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// 转写视图：按事件顺序（append-only ⇒ 时间序）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptView {
    pub entries: Vec<TranscriptEntry>,
}

/// 一条转写条目的渲染行（角色前缀 + 正文）。
///
/// 工具行用 `工具结果(<tool>): ` 前缀——与轮内交换（`render_tool_exchange`）
/// 逐字同形；用户 / 助手行沿用 `用户: ` / `助手: `。
fn render_line(e: &TranscriptEntry) -> String {
    match e.role.as_str() {
        "user" => format!("用户: {}", e.text),
        "tool" => format!("工具结果({}): {}", e.tool.as_deref().unwrap_or(""), e.text),
        _ => format!("助手: {}", e.text),
    }
}

impl TranscriptView {
    /// 平铺成单 prompt（多轮历史 + 当前消息）。单轮 ⇒ 裸文本（与无历史形态等价）。
    pub fn to_prompt(&self) -> String {
        match self.entries.as_slice() {
            [] => String::new(),
            [only] => only.text.clone(),
            many => {
                let history = &many[..many.len() - 1];
                let current = &many[many.len() - 1];
                let mut out = String::from("<对话历史>\n");
                for e in history {
                    out.push_str(&render_line(e));
                    out.push('\n');
                }
                out.push_str("</对话历史>\n");
                out.push_str(&render_line(current));
                out
            }
        }
    }
}

/// `transcript` 投影：从事件切片读出对话转写。
pub fn transcript() -> Projection<TranscriptView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mut entries = Vec::new();
        for e in events {
            if e.ts > now {
                continue;
            }
            match (e.entity, e.verb, e.kind.as_str()) {
                (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "user".into(),
                        text: text.to_string(),
                        tool: None,
                    });
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: text.to_string(),
                        tool: None,
                    });
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => {
                    let why = e.payload.get("why").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: why.to_string(),
                        tool: None,
                    });
                }
                // 工具结果（`artifact × asserted`）：跨轮可见的「我调用过什么、拿到了什么」。
                (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => {
                    let tool = e.payload.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "tool".into(),
                        text: text.to_string(),
                        tool: Some(tool.to_string()),
                    });
                }
                _ => {}
            }
        }
        View {
            value: TranscriptView { entries },
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}
