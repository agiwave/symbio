//! Session 类型定义

use serde::{Deserialize, Serialize};

pub use crate::symbio_core::schemas::session::chat_message::ChatMessage;

/// 会话数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: i64,
    pub updated_at: i64,
    pub metadata: serde_json::Value,
}

impl Session {
    pub fn new(id: impl Into<String>) -> Self {
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        Self {
            id: id.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
        }
    }

    /// 会话显示名：metadata.title 优先；否则从内容自动生成；再否则回落 id。
    ///
    /// 「从内容生成」= 第一条含文本的用户消息首行（压缩空白、限长），见
    /// [`derive_session_title`]。列表（entities/list）与本方法共用同一规则，
    /// 保证自动命名会话在任何入口看到的名称一致。
    pub fn display_title(&self) -> String {
        self.metadata
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .or_else(|| derive_session_title(&self.messages))
            .unwrap_or_else(|| "新对话".to_string())
    }
}

/// 从会话消息内容自动生成会话标题（无显式命名时的兜底）。
///
/// 规则：第一条**含文本**的用户消息 → 首行 → 压缩连续空白 → 限长
/// [`SESSION_TITLE_MAX_CHARS`] 字符（超长追加省略号）。找不到文本时返回 None。
pub(crate) fn derive_session_title(messages: &[ChatMessage]) -> Option<String> {
    const SESSION_TITLE_MAX_CHARS: usize = 24;
    for m in messages {
        if !matches!(m.role, Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User)) {
            continue;
        }
        let text = match &m.content {
            Some(crate::symbio_core::schemas::session::chat_message::MessageContent::Text(s)) => s.clone(),
            Some(crate::symbio_core::schemas::session::chat_message::MessageContent::Parts(parts)) => parts
                .iter()
                .filter_map(|p| match p {
                    crate::symbio_core::schemas::session::chat_message::ContentPart::Text { text } => {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
            None => continue,
        };
        // 首行 + 压缩空白 + 限长
        let first_line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
        let mut out = String::new();
        let mut chars = first_line.chars();
        for _ in 0..SESSION_TITLE_MAX_CHARS {
            match chars.next() {
                Some(c) if c.is_whitespace() => {
                    if !out.ends_with(' ') {
                        out.push(' ');
                    }
                }
                Some(c) => out.push(c),
                None => break,
            }
        }
        let truncated = out.trim_end();
        return Some(if chars.next().is_some() {
            format!("{truncated}…")
        } else {
            truncated.to_string()
        });
    }
    None
}

/// 会话心跳任务配置
///
/// 存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。
/// 后端 [`crate::plugins::session::SessionPlugin`] 的后台调度器据此在会话空闲
/// 指定时间后自动发起一次提示词对话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// 是否启用心跳任务
    #[serde(default)]
    pub enabled: bool,
    /// 启动间隔（秒）：会话空闲达到该时长后触发一次心跳
    #[serde(default = "default_heartbeat_interval")]
    pub interval_seconds: u64,
    /// 心跳任务提示词（每次触发时作为一条用户消息发送给模型）
    #[serde(default)]
    pub prompt: String,
    /// 启动心跳时是否携带历史会话信息
    /// - `true`（默认）：心跳消息作为普通对话追加，模型能看到历史
    /// - `false`：本次发送不加载任何历史（"无上下文"心跳）
    #[serde(default = "default_heartbeat_include_history")]
    pub include_history: bool,
}

fn default_heartbeat_interval() -> u64 {
    300
}

fn default_heartbeat_include_history() -> bool {
    true
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: default_heartbeat_interval(),
            prompt: String::new(),
            include_history: default_heartbeat_include_history(),
        }
    }
}

impl HeartbeatConfig {
    /// 从会话 metadata 解析心跳配置。字段缺失时返回默认（未启用）配置。
    pub fn from_metadata(metadata: &serde_json::Value) -> Self {
        let Some(obj) = metadata.get("heartbeat").and_then(|v| v.as_object()) else {
            return Self::default();
        };
        let enabled = obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let interval_seconds = obj
            .get("interval_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(default_heartbeat_interval);
        let prompt = obj
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let include_history = obj
            .get("include_history")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(default_heartbeat_include_history);
        Self {
            enabled,
            interval_seconds,
            prompt,
            include_history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_metadata_missing_returns_disabled_default() {
        let m = serde_json::json!({ "title": "x" });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(!hb.enabled);
        assert_eq!(hb.interval_seconds, 300);
        assert!(hb.include_history);
        assert!(hb.prompt.is_empty());
    }

    #[test]
    fn from_metadata_parses_full_config() {
        let m = serde_json::json!({
            "heartbeat": {
                "enabled": true,
                "interval_seconds": 120,
                "prompt": "请检查待办",
                "include_history": false
            }
        });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(hb.enabled);
        assert_eq!(hb.interval_seconds, 120);
        assert_eq!(hb.prompt, "请检查待办");
        assert!(!hb.include_history);
    }

    #[test]
    fn from_metadata_missing_fields_use_defaults() {
        let m = serde_json::json!({ "heartbeat": { "enabled": true } });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(hb.enabled);
        assert_eq!(hb.interval_seconds, 300);
        assert!(hb.include_history);
    }

    fn user_msg(text: &str) -> ChatMessage {
        use crate::symbio_core::schemas::session::chat_message as cm;
        ChatMessage {
            id: "m1".into(),
            role: Some(cm::MessageRole::User),
            content: Some(cm::MessageContent::Text(text.into())),
            ..Default::default()
        }
    }

    #[test]
    fn derive_title_takes_first_user_line() {
        let msgs = vec![user_msg("帮我分析一下这个报错\n第二行"), user_msg("第二条")];
        assert_eq!(derive_session_title(&msgs).as_deref(), Some("帮我分析一下这个报错"));
    }

    #[test]
    fn derive_title_truncates_long_text() {
        let long = "这是一个特别特别长的用户首条消息用来验证截断逻辑是否正常工作";
        let t = derive_session_title(&[user_msg(long)]).unwrap();
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 25);
    }

    #[test]
    fn derive_title_skips_empty_and_non_user() {
        use crate::symbio_core::schemas::session::chat_message as cm;
        let empty = ChatMessage {
            id: "m0".into(),
            role: Some(cm::MessageRole::User),
            content: Some(cm::MessageContent::Text("   \n  ".into())),
            ..Default::default()
        };
        assert_eq!(derive_session_title(&[empty]), None);
    }

    #[test]
    fn display_title_prefers_metadata_then_content_then_default() {
        let mut s = Session::new("sess-123");
        assert_eq!(s.display_title(), "新对话");
        s.messages = vec![user_msg("帮我查天气")];
        assert_eq!(s.display_title(), "帮我查天气");
        s.metadata = serde_json::json!({ "title": "自定义名" });
        assert_eq!(s.display_title(), "自定义名");
    }
}
