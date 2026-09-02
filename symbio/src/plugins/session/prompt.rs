//! 会话基础提示词装配（**与智能体无关**）
//!
//! ## 重构背景
//!
//! 重构前，`AGENTS.md`（全局 / 工作区指令）与 `<context>` 时间上下文都由
//! **agent 插件**在 `agent/chat` 里注入 system prompt。这导致一个荒谬的结果：
//! 一个不想用智能体的会话，连工作区指令都读不到。
//!
//! 这两样东西本质上都是**工作区 / 环境级**的，与"是否选择智能体"正交，
//! 因此归属 session 插件——会话编排方本就该负责环境上下文。
//!
//! ## 本模块与人格的分工
//!
//! | 内容 | 归属 | 送达方式 |
//! |---|---|---|
//! | 全局 `AGENTS.md` | session（本模块） | system prompt |
//! | 工作区 `AGENTS.md` | session（本模块） | system prompt |
//! | 时间 / 工作区 `<context>` | session（本模块） | 用户消息 `prompt` 前缀 |
//! | 智能体身份 / 规则 / 策略 | agent 插件 | `agent_identity` 工具说明 |
//!
//! 智能体相关内容一律不在本模块出现——session 不知道"人格"这个概念。
//!
//! ## 时间上下文为什么挂在用户消息上
//!
//! 每轮发送都重新生成，挂在 `msg.prompt` 上会让**历史轮次保留当时的时间戳**，
//! 比塞进 system prompt（每轮被覆盖）更能反映真实的对话时序。

use std::path::Path;

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

/// 组装会话基础系统提示词（全局指令 + 工作区指令）
///
/// 任一指令文件缺失时该段整体省略；两者都缺失时返回空串，
/// 由调用方回退到模型插件的默认提示词。
pub async fn build_system_prompt(workdir: Option<&str>) -> String {
    let mut buf = String::new();

    if let Some(system_agents) =
        read_to_string_safe(&crate::symbio_core::HomedirRegistry::get().join("AGENTS.md")).await
    {
        if !system_agents.trim().is_empty() {
            buf.push_str("## 全局指令\n");
            buf.push_str(system_agents.trim_end());
            buf.push_str("\n\n");
        }
    }

    if let Some(workspace_agents) = read_workspace_agents_md(workdir).await {
        if !workspace_agents.trim().is_empty() {
            buf.push_str("## 工作区指令\n");
            buf.push_str(workspace_agents.trim_end());
            buf.push_str("\n\n");
        }
    }

    buf
}

/// 读取 `{workdir}/AGENTS.md`
///
/// 安全说明：拒绝相对路径与含 `..` 的路径，避免工作区指令文件读取逃逸出
/// 用户声明的目录。workdir 由上层（home 插件 `set_workspace`）规范化为绝对路径。
async fn read_workspace_agents_md(workdir: Option<&str>) -> Option<String> {
    let wd = workdir?.trim();
    if wd.is_empty() {
        return None;
    }
    let path = Path::new(wd);
    if !path.is_absolute() {
        crate::plugin_warn!("session", "拒绝读取非绝对工作区路径的 AGENTS.md: {}", wd);
        return None;
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        crate::plugin_warn!("session", "拒绝读取含 `..` 的工作区路径 AGENTS.md: {}", wd);
        return None;
    }
    read_to_string_safe(&path.join("AGENTS.md")).await
}

async fn read_to_string_safe(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    tokio::fs::read_to_string(path).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_context_contains_time() {
        let result = temporal_context(Some("/workspace"));
        assert!(result.contains("<context>"), "应包含 context 标签");
        assert!(result.contains("工作区: /workspace"), "应包含工作区路径");
        assert!(result.contains("星期"), "应包含星期信息");
    }

    #[test]
    fn temporal_context_without_workdir() {
        let result = temporal_context(None);
        assert!(result.contains("<context>"), "应包含 context 标签");
        assert!(!result.contains("工作区"), "无 workdir 时不应包含工作区");
    }

    #[test]
    fn temporal_context_ignores_empty_workdir() {
        let result = temporal_context(Some("   "));
        assert!(!result.contains("工作区"), "空白 workdir 应被忽略");
    }

    #[tokio::test]
    async fn empty_workdir_yields_no_instructions() {
        // 无 workdir 时不应 panic，且工作区指令段缺失
        let prompt = build_system_prompt(None).await;
        assert!(
            !prompt.contains("## 工作区指令"),
            "无 workdir 时不应有工作区指令段"
        );
    }

    #[test]
    fn relative_workdir_is_rejected() {
        // 相对路径不得被解析为工作区指令路径
        assert!(Path::new("relative/dir").is_absolute() == false);
    }
}
