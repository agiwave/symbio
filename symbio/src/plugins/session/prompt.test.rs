//! `session/prompt.rs` 的单元测试 —— 时间上下文。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 这里同时钉住**一次移除**：本模块不得再出现任何 `AGENTS.md` 的读取——
//! 智能体自身那个文件归 `setting` 插件，工作区那个归 `work` 插件。

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
