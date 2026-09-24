//! `symbio/src/symbio_core/vdfs/address.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 拼接规则：单级 / 多级 / 两边多余分隔符 / 无父可接
#[test]
fn join_addr_normalizes_both_sides() {
    assert_eq!(join_addr("@root", ""), "@root");
    assert_eq!(join_addr("@root/", "/"), "@root");
    assert_eq!(
        join_addr("@root/session", "abc/AGENTS.md"),
        "@root/session/abc/AGENTS.md"
    );
    assert_eq!(join_addr("@root/session/", "/abc/"), "@root/session/abc");
    assert_eq!(join_addr("", "session/x"), "session/x");
    assert_eq!(join_addr("  ", "session/x"), "session/x");
}

/// 静态声明可达（本 crate 编译进了 vdfs 插件，它提交了根名）。
/// 具体的名字归那个插件——这里只钉「声明机制在工作」。
#[test]
fn declaration_is_reachable_and_normalized() {
    let root = declared_addr_root().expect("vdfs 插件应已静态声明根名");
    assert!(!root.is_empty());
    assert!(
        !root.ends_with('/') && !root.starts_with('/'),
        "应已归一化: {root}"
    );
}

/// 转发跳的改写规则：顶层落到声明的根；嵌套时在当前父地址上续接
#[test]
fn descend_starts_at_root_then_layers() {
    let root = declared_addr_root().unwrap();
    // 顶层（转发方自己没有父地址）→ 根 + 本级名
    assert_eq!(descend_addr("", "session"), format!("{root}/session"));
    assert_eq!(descend_addr("  ", "session"), format!("{root}/session"));
    // 嵌套（转发方已有父地址）→ 原地续接，不回落到根
    assert_eq!(
        descend_addr("@elsewhere/agent/b1", "sub"),
        "@elsewhere/agent/b1/sub"
    );
}
