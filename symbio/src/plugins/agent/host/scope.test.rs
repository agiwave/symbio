//! `symbio/src/plugins/agent/host/scope.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn tool_name_uses_protocol_safe_chars() {
    // agent id 允许 `.`（工具名不许，进 function-calling 协议）→ 压成 `_`；
    // `-` 在协议允许集内（`[A-Za-z0-9_-]`），保留以维持可读。
    let v = SubAgentVisitor::new(
        Arc::new(crate::providers::collectors::DefaultToolVisitor::new()),
        "com.acme.code-reviewer",
    );
    assert_eq!(
        v.tool("read_skill"),
        "agent_com_acme_code-reviewer_read_skill"
    );
    assert!(
        v.tool("read_skill")
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "工具名必须落在 function-calling 的字符集内"
    );
    assert_eq!(v.seg("work"), "agent/com.acme.code-reviewer/work");
}

#[test]
fn seg_keeps_id_readable() {
    let v = SubAgentVisitor::new(
        Arc::new(crate::providers::collectors::DefaultToolVisitor::new()),
        "reviewer",
    );
    assert_eq!(v.seg("work"), "agent/reviewer/work");
    assert_eq!(v.tool("read_skill"), "agent_reviewer_read_skill");
}
