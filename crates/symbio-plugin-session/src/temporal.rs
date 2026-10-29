//! 时间上下文：挂在用户消息 `prompt` 字段上的一段 `<context>`。
//!
//! 留在 session 的原因：它的唯一调用点是 `orchestrator/entry.rs`（用户消息落库前
//! 填 `prompt`），落在驱动器搬家面之外。

use time::{format_description, OffsetDateTime};

/// 渲染时间 / 工作区上下文（每轮生成）。
///
/// ## 这里不读任何 `AGENTS.md`
///
/// `{homedir}/AGENTS.md` 与 `<agentdir>/AGENTS.md` 都归 `memory` 插件
/// （见 `crates/symbio-plugin-memory/src/memory.rs`），`work` 只认 `{workdir}`。
///
/// ## 时间上下文为什么挂在用户消息上
///
/// 每轮发送都重新生成，挂在 `msg.prompt` 上会让**历史轮次保留当时的时间戳**，
/// 比塞进 system prompt（每轮被覆盖）更能反映真实的对话时序。
pub fn temporal_context(workdir: Option<&str>) -> String {
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
#[path = "temporal.test.rs"]
mod tests;
