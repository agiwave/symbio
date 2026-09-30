//! `transcript` 投影——从事件网格读出**对话转写**（v2 会话运行时的历史侧）。
//!
//! [`crate::symbio_core::actors::Reasoner`] 组 prompt 的输入：多轮对话时模型
//! 需要看到自己说过什么（否则第二轮起失忆）。转写是**纯函数读视图**——
//! 历史来自同一份事实源，不另存副本（ADR-044：实测与判据同源的同族纪律）。
//!
//! 口径：
//! - `user.message`（`turn × opened`）→ `user` 行（载荷 `text`）；
//! - `chat.assistant.final`（`turn × closed`）→ `assistant` 行（载荷 `text`）；
//! - `chat.assistant.fallback`（`turn × closed`）→ `assistant` 行（载荷 `why`）——
//!   兜底话术是用户实际看到的回复，模型必须知道它「说」过这个。

use super::super::event::{
    Entity, Event, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};

/// 转写条目：一句话及其角色。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// `"user"` 或 `"assistant"`。
    pub role: String,
    pub text: String,
}

/// 转写视图：按事件顺序（append-only ⇒ 时间序）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptView {
    pub entries: Vec<TranscriptEntry>,
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
                    if e.role == "user" {
                        out.push_str("用户: ");
                    } else {
                        out.push_str("助手: ");
                    }
                    out.push_str(&e.text);
                    out.push('\n');
                }
                out.push_str("</对话历史>\n");
                if current.role == "user" {
                    out.push_str("用户: ");
                } else {
                    out.push_str("助手: ");
                }
                out.push_str(&current.text);
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
            if e.ts > now || e.entity != Entity::Turn {
                continue;
            }
            match (e.verb, e.kind.as_str()) {
                (Verb::Opened, EVENT_USER_MESSAGE) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "user".into(),
                        text: text.to_string(),
                    });
                }
                (Verb::Closed, EVENT_ASSISTANT_FINAL) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: text.to_string(),
                    });
                }
                (Verb::Closed, EVENT_ASSISTANT_FALLBACK) => {
                    let why = e.payload.get("why").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: why.to_string(),
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
