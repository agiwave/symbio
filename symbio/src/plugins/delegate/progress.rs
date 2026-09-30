//! worker 会话状态快照 —— **纯函数 + 磁盘公开布局**：主会话回答"进展如何"的事实源。
//!
//! ## 数据从哪来（不许有第二份状态）
//!
//! worker 是**独立会话历史**：按 `metadata.parent_session_id` 归档在
//! `<root>/session/<父>/sessions/<子>/`（嵌套布局是 `session` 存储的公开约定，
//! 见其 store 模块文档）。因此本模块**只读磁盘**，不向 `session` 要任何接口——
//! 与 `fact_log` / `retrieval` 同款（插件独立原则）。
//!
//! **不做进程内注册表**：worker 的"进展"若另存一份，就是第二份真相——主会话
//! 与服务重启后的状态会对不上（这正是 v2 I1「一切协作经事实源」的反面）。
//!
//! ## 快照怎么算出来（全部可从消息推出，不引入新字段）
//!
//! | 字段 | 推导 |
//! |---|---|
//! | `rounds` | 用户消息条数（"跑了几个来回"） |
//! | `state` | 仍存在 `streaming` / `pending` 的消息 ⇒ `running`；否则 `idle` |
//! | `last_step` | 末条消息：工具调用 ⇒ 工具名；助手文本 ⇒ 截断摘要；否则 `—` |
//! | `title` | `session.json` 的 `title` 投影；缺失则回落到首个用户消息 |
//!
//! ## 平凡值与确定性
//!
//! - 没有 worker 会话 ⇒ 空表；[`render`] 返回**空串**（调用方不注入空段落）；
//! - 输出按会话 id 字典序（排序保证），可双跑比对（A4）；
//! - 任何单个会话读不动（缺文件 / 坏 JSON）⇒ **跳过它**，不让一处坏数据毁掉整段快照。

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 会话清单文件名（`session` 存储的公开约定）
const SESSION_FILE: &str = "session.json";
/// 消息文件名（同上）
const MESSAGES_FILE: &str = "messages.json";
/// 子会话子目录名（同上：`<父会话目录>/sessions/<子会话>/`）
const SUB_SESSIONS_DIR: &str = "sessions";
/// 快照标题（注入进提示词时的开头；无 worker 时整段不出现）
pub const PROGRESS_TITLE: &str = "【后台任务】";
/// `last_step` 摘要的字符上限
const STEP_MAX_CHARS: usize = 60;

/// 一个 worker 会话的状态快照
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerState {
    pub session_id: String,
    /// 父会话 id（= 主会话）。**归属靠它而非目录位置**：同一主会话的 worker 才是
    /// "我的后台任务"，别的会话的不该出现在本会话的注入段里（串台 = 说错话）。
    pub parent: String,
    pub title: String,
    /// 用户消息条数
    pub rounds: usize,
    /// `running`（仍有在途消息）/ `idle`（已收敛）
    pub state: &'static str,
    /// 最新一步的摘要（工具名 / 助手文本片段 / `—`）
    pub last_step: String,
}

/// 扫描 `<root>/session/*/sessions/*/` 推出**全部** worker 快照（按会话 id 升序）。
///
/// 归属由 `parent_session_id` 判据保留在快照里（没有它的目录不是 worker，跳过）。
pub fn scan(root: &Path) -> Vec<WorkerState> {
    scan_filtered(root, None)
}

/// 只推出**某一个主会话名下**的 worker（主会话注入自己那一段时用）。
///
/// 父 id 为空串 ⇒ 空表（不是"全部"）：宁可什么都不说，也不把别人的后台任务
/// 当成自己的——注入串台比没有注入更糟。
pub fn scan_of(root: &Path, parent: &str) -> Vec<WorkerState> {
    let p = parent.trim();
    if p.is_empty() {
        return Vec::new();
    }
    scan_filtered(root, Some(p))
}

fn scan_filtered(root: &Path, parent_filter: Option<&str>) -> Vec<WorkerState> {
    let session_root = root.join(super::plugin::PLUGIN_ID_SESSION_DIR);
    let mut out: Vec<WorkerState> = Vec::new();
    let Ok(parents) = std::fs::read_dir(&session_root) else {
        return out;
    };
    for parent in parents.flatten() {
        let sub_root: PathBuf = parent.path().join(SUB_SESSIONS_DIR);
        let Ok(children) = std::fs::read_dir(&sub_root) else {
            continue;
        };
        for child in children.flatten() {
            let dir = child.path();
            if !dir.is_dir() {
                continue;
            }
            if let Some(state) = snapshot_of(&dir) {
                if parent_filter.is_none_or(|p| p == state.parent) {
                    out.push(state);
                }
            }
        }
    }
    out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    out
}

/// 读单个 worker 目录推出快照；不可读 / 缺消息 ⇒ `None`（跳过，不报错）
fn snapshot_of(dir: &Path) -> Option<WorkerState> {
    let meta = read_meta(dir)?;
    // 必须是**子会话**：没有 parent_session_id 的目录不是 worker（防御性判据）
    let parent = meta
        .get("metadata")
        .and_then(|m| m.get("parent_session_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let session_id = dir.file_name()?.to_str()?.to_string();
    if parent == session_id {
        return None;
    }

    let messages = read_messages(dir, &meta)?;
    let rounds = messages
        .iter()
        .filter(|m| m.role == Some(MessageRole::User))
        .count();
    let running = messages.iter().any(|m| {
        matches!(
            m.status,
            Some(MessageStatus::Streaming) | Some(MessageStatus::Pending)
        )
    });
    let title = meta
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| first_user_text(&messages));

    Some(WorkerState {
        session_id,
        parent: parent.to_string(),
        title,
        rounds,
        state: if running { "running" } else { "idle" },
        last_step: last_step_of(&messages),
    })
}

/// 读 `session.json`（不存在 / 坏 JSON ⇒ `None`）
fn read_meta(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join(SESSION_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 读消息（只声明本模块要用的那个键，同 `retrieval`）。
///
/// **老布局回退**：`session.json` 内联过 `messages`（拆分存储之前的历史数据），
/// 只有 `messages.json` 的会话会被这条回退救回来；`session` 存储自己读消息也是
/// 同样的两级顺序。宁可直接读内联数组，也不让"看得到目录、看不到进展"。
fn read_messages(dir: &Path, meta: &Value) -> Option<Vec<ChatMessage>> {
    if let Ok(text) = std::fs::read_to_string(dir.join(MESSAGES_FILE)) {
        if let Ok(file) = serde_json::from_str::<MessagesFile>(&text) {
            return Some(file.messages);
        }
    }
    let inline = meta.get("messages")?.as_array()?.clone();
    serde_json::from_value(Value::Array(inline)).ok()
}

/// `messages.json` 的落盘形状 —— 就地重声明一个只读键，不 import `session`
#[derive(serde::Deserialize)]
struct MessagesFile {
    #[serde(default)]
    messages: Vec<ChatMessage>,
}

/// 最新一步的摘要：工具调用优先（"在用什么手段"比"在说什么"更能反映进展）
fn last_step_of(messages: &[ChatMessage]) -> String {
    for m in messages.iter().rev() {
        if m.msg_type == Some(MessageType::ToolCall) {
            if let Some(name) = m.name.as_deref().filter(|s| !s.trim().is_empty()) {
                return format!("调用工具 {name}");
            }
        }
        if m.role == Some(MessageRole::Assistant) {
            let text = text_of(m);
            if !text.trim().is_empty() {
                return truncate_chars(text.trim(), STEP_MAX_CHARS);
            }
        }
    }
    "—".to_string()
}

/// 首个用户消息的文本（标题回落）
fn first_user_text(messages: &[ChatMessage]) -> String {
    for m in messages {
        if m.role == Some(MessageRole::User) {
            let t = text_of(m);
            if !t.trim().is_empty() {
                return truncate_chars(t.trim(), STEP_MAX_CHARS);
            }
        }
    }
    "未命名任务".to_string()
}

/// 取消息正文（只认文本内容；结构化内容不进摘要）
fn text_of(m: &ChatMessage) -> &str {
    match m.content.as_ref() {
        Some(MessageContent::Text(t)) => t,
        _ => "",
    }
}

/// 按**字符**（不是字节）截断——中文被按字节切会 panic，这条在全仓已有先例
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// 渲染成提示词段落；**空表 ⇒ 空串**（调用方据此不注入空标题）
pub fn render(states: &[WorkerState]) -> String {
    if states.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(PROGRESS_TITLE);
    out.push('\n');
    for s in states {
        let title = if s.title.trim().is_empty() {
            "未命名任务"
        } else {
            s.title.trim()
        };
        out.push_str(&format!(
            "- {}（{}）：{} · 第 {} 轮 · 最新一步 {}\n",
            s.session_id,
            title,
            if s.state == "running" {
                "运行中"
            } else {
                "已收敛"
            },
            s.rounds,
            s.last_step
        ));
    }
    out
}

#[cfg(test)]
#[path = "progress.test.rs"]
mod tests;
