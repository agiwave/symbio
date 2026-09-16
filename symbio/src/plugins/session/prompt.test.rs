//! `session/prompt.rs` 的单元测试 —— 时间上下文与全局指令。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 这里同时钉住**一次移除**：工作区 `AGENTS.md` 不得再出现在本模块的产物里
//! （那是 work 插件的工作区记忆，见模块文档的归属说明）。

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

/// 全局指令**没有**地址与容量：宿主不提供它的编辑面，假装有只会误导模型
#[test]
fn global_instruction_has_no_address_or_capacity() {
    let rendered = format!("【{GLOBAL_TITLE}】（{AGENTS_FILE}，对所有会话生效）\n正文\n");
    assert!(rendered.contains("【全局指令】"));
    assert!(!rendered.contains(".vdfs"), "全局指令不该带 VDFS 地址");
    assert!(!rendered.contains("上限"), "全局指令不该带容量口径");
}

/// 注册名是覆盖键，不是分类——改名会让「同名覆盖」失效
#[test]
fn global_prompt_name_is_stable() {
    assert_eq!(GLOBAL_PROMPT_NAME, "session-global-instructions");
}
