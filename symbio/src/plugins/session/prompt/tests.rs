//! `prompt` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `prompt.rs` 只保留生产代码，测试全部放本文件。

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
    assert!(!Path::new("relative/dir").is_absolute());
}
