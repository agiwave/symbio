//! 会话基础提示词装配 —— **只剩时间上下文**。
//!
//! ## 这里不再读任何 `AGENTS.md`
//!
//! 本模块曾读 `{homedir}/AGENTS.md` 当「全局指令」注入系统提示词。那是一次**越界**：
//! 文件不在会话的作用域里（它属于**智能体自身目录**），宿主也不给它编辑面，
//! 于是同一件事被两个所有者用两种口径各做一遍——`session` 读只读版本、子 Agent 树里
//! 的 `work` 实例读可写版本。
//!
//! 现在按 **作用域归所有者** 收口：`{homedir}/AGENTS.md` 与 `<agentdir>/AGENTS.md`
//! 都归 `agent` 插件（见 `plugins/agent/host/instruction.rs`），`work` 只认 `{workdir}`。
//!
//! ## 时间上下文为什么挂在用户消息上
//!
//! 每轮发送都重新生成，挂在 `msg.prompt` 上会让**历史轮次保留当时的时间戳**，
//! 比塞进 system prompt（每轮被覆盖）更能反映真实的对话时序。

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

#[cfg(test)]
#[path = "prompt.test.rs"]
mod tests;
