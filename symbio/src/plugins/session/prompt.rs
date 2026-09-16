//! 会话基础提示词装配（**与智能体、与工作区都无关**）。
//!
//! ## 全局指令归本模块；工作区指令**不归**
//!
//! `{homedir}/AGENTS.md` 是「用户写给**所有**会话的规则」：只读、无地址、无容量，
//! 模型改不动它——它**不是记忆**（判据见 `symbio_core::memory` 的模块文档：
//! 内核只收「模型能自己改的东西」）。本模块把它读出来，经
//! `register_system_prompt` 注入。
//!
//! `{workdir}/AGENTS.md` **不在这里**——那是 work 插件的**工作区记忆**
//!（可读写、有 VDFS 地址、有容量闸门）。本模块曾经把它当「工作区指令」再读一遍、
//! 注入系统提示词基座，结果是同一份内容进两次上下文，而模型改不动它。
//!
//! 现在的约定是 **谁能读写它，谁负责注入它**——一个作用域只有一个所有者。
//!
//! ## 为什么全局指令走注册通道而不是 `req.system_prompt`
//!
//! 后者是**显式提示词**（调用方直接给的），一旦填入就会**顶掉**模型插件注册的人格
//! ——「用户写了一份全局指令」不该导致「模型换了个人格」。走注册通道则与人格并列，
//! 两者同时在场（见 `chat_loop::inputs::resolve_system_prompt`）。
//!
//! ## 时间上下文为什么挂在用户消息上
//!
//! 每轮发送都重新生成，挂在 `msg.prompt` 上会让**历史轮次保留当时的时间戳**，
//! 比塞进 system prompt（每轮被覆盖）更能反映真实的对话时序。

use crate::symbio_core::AGENTS_FILE;
use std::path::Path;

/// 全局指令在能力收集器里的注册名（同名覆盖的键）
pub const GLOBAL_PROMPT_NAME: &str = "session-global-instructions";

/// 全局指令的标题（渲染为 `【全局指令】`）
pub const GLOBAL_TITLE: &str = "全局指令";

/// 渲染时间 / 工作区上下文（每轮生成）
pub fn temporal_context(workdir: Option<&str>) -> String {
    use time::{format_description, OffsetDateTime};

    let now = OffsetDateTime::now_utc();
    // time 0.3.55 起 parse 被 deprecated，parse_borrowed::<2> 为等价替代
    let fmt = format_description::parse_borrowed::<2>("[year]-[month]-[day] [hour]:[minute]")
        .unwrap_or_default();
    let time_str = now.format(&fmt).unwrap_or_else(|_| "unknown".to_string());

    let weekday = match now.weekday() {
        time::Weekday::Monday => "星期一",
        time::Weekday::Tuesday => "星期二",
        time::Weekday::Wednesday => "星期三",
        time::Weekday::Thursday => "星期四",
        time::Weekday::Friday => "星期五",
        time::Weekday::Saturday => "星期六",
        time::Weekday::Sunday => "星期日",
    };

    let mut line = format!("<context>当前时间: {} {}", time_str, weekday);
    if let Some(wd) = workdir.map(str::trim).filter(|w| !w.is_empty()) {
        line.push_str(&format!(" | 工作区: {}", wd));
    }
    line.push_str("</context>\n");
    line
}

/// 读取并渲染**全局指令**（`{homedir}/AGENTS.md`）。
///
/// 文件不存在 / 为空 → `None`（该段整体省略，不产出空标题）。
///
/// 形态对齐记忆层：一行头信息 + 正文。但它**没有地址与容量**——这是刻意的，
/// 因为宿主不提供它的编辑面（用户在文件系统里改），假装有地址只会误导模型。
pub async fn global_instruction() -> Option<String> {
    let path = crate::symbio_core::HomedirRegistry::get().join(AGENTS_FILE);
    let text = read_to_string_safe(&path).await?;
    if text.trim().is_empty() {
        return None;
    }
    Some(format!(
        "【{GLOBAL_TITLE}】（{AGENTS_FILE}，对所有会话生效）\n{}\n",
        text.trim_end()
    ))
}

async fn read_to_string_safe(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    tokio::fs::read_to_string(path).await.ok()
}

#[cfg(test)]
#[path = "prompt.test.rs"]
mod tests;
