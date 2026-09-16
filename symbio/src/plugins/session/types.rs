//! Session 数据类型：`Session` 实体 + 标题推导 + 心跳配置。
//!
//! 注意与 `core/schemas/session/chat_message.rs`（消息级类型）区分：本文件
//! 描述"会话"粒度的持久化实体。`derive_session_title` 供会话创建时兜底命名，
//! `HeartbeatConfig` 从会话 metadata 派生心跳调度参数（消费者 `heartbeat.rs`）。

use serde::{Deserialize, Serialize};

pub use crate::symbio_core::schemas::session::chat_message::ChatMessage;

/// 会话列表一行摘要：最后一条含文本消息的首行（压缩空白、限长 60 字符）。
///
/// 与 [`derive_session_title`] 同风格；供会话节点的 `description` 驱动列表
/// 「实时缩略」预览。**清单投影**的一部分（见 [`SessionSummary`]）。
pub fn derive_session_summary(messages: &[ChatMessage]) -> Option<String> {
    const SUMMARY_MAX_CHARS: usize = 60;
    let text = messages
        .iter()
        .rev()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_text()))
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())?;
    let first_line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out = String::new();
    let mut chars = first_line.chars();
    for _ in 0..SUMMARY_MAX_CHARS {
        match chars.next() {
            Some(c) if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            Some(c) => out.push(c),
            None => return Some(out.trim_end().to_string()),
        }
    }
    Some(format!("{}…", out.trim_end()))
}

/// 会话的通用元信息标签（工作目录名 + 消息数）。
///
/// 标签由本函数单点产出，挂在 VDFS 节点的 `attributes.meta_tags` 上，保证同一
/// 会话在清单与详情里的呈现一致；前端原样渲染，不含语义。
pub fn session_meta_tags(metadata: &serde_json::Value, message_count: usize) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    if let Some(wd) = metadata
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let base = wd
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(wd);
        if !base.is_empty() {
            tags.push(base.to_string());
        }
    }
    tags.push(format!("{message_count} 条"));
    tags
}

/// 会话清单项 —— **不含消息**。
///
/// ## 为什么需要它
///
/// 清单（`.vdfs/session` 的 `list`）必须**不读** `messages.json`（那正是存储拆分
/// 的全部收益），而清单要显示的字段——标题 / 条数 / 摘要 / 标签——原本**全是从
/// 消息算出来的**。所以这些字段在 `save` 时算好、随元数据落盘：它们是可重算的
/// **投影**，不是第二份真相，计算入口只有 [`SessionSummary::of`] 一个。
///
/// 类型上**不提供** `messages`，是为了让「清单路径又去碰消息」这件事**编译不过**；
/// 返回「messages 为空的 `Session`」做不到这点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    /// `display_title()` 的落盘快照
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub metadata: serde_json::Value,
    pub message_count: usize,
    pub summary: Option<String>,
    pub meta_tags: Vec<String>,
}

impl SessionSummary {
    /// 从完整会话算出投影 —— **唯一**计算入口（`save` 时调用；清单只读结果）。
    pub fn of(s: &Session) -> Self {
        Self {
            id: s.id.clone(),
            title: s.display_title(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            metadata: s.metadata.clone(),
            message_count: s.messages.len(),
            summary: derive_session_summary(&s.messages),
            meta_tags: session_meta_tags(&s.metadata, s.messages.len()),
        }
    }
}

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
    /// [`derive_session_title`]。列表（`vdfs/list`）与本方法共用同一规则，
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

    /// 父会话 id（`metadata.parent_session_id`；空/自引用视为无归属）。
    ///
    /// 子会话归属的机制级声明：存储层据此路由嵌套目录（文件后端），
    /// 会话 provider 据此判定「子会话」清单归属。
    pub fn parent_session_id(&self) -> Option<&str> {
        self.metadata
            .get("parent_session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|p| !p.is_empty() && *p != self.id)
    }
}

/// 从会话消息内容自动生成会话标题（无显式命名时的兜底）。
///
/// 规则：第一条**含文本**的用户消息 → 首行 → 压缩连续空白 → 限长
/// [`SESSION_TITLE_MAX_CHARS`] 字符（超长追加省略号）。找不到文本时返回 None。
pub(crate) fn derive_session_title(messages: &[ChatMessage]) -> Option<String> {
    const SESSION_TITLE_MAX_CHARS: usize = 24;
    for m in messages {
        if !matches!(
            m.role,
            Some(crate::symbio_core::schemas::session::chat_message::MessageRole::User)
        ) {
            continue;
        }
        let text = match &m.content {
            Some(crate::symbio_core::schemas::session::chat_message::MessageContent::Text(s)) => {
                s.clone()
            }
            Some(crate::symbio_core::schemas::session::chat_message::MessageContent::Parts(
                parts,
            )) => parts
                .iter()
                .filter_map(|p| match p {
                    crate::symbio_core::schemas::session::chat_message::ContentPart::Text {
                        text,
                    } => Some(text.clone()),
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
mod tests;
