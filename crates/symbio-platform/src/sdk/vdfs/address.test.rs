//! `crates/symbio-platform/src/sdk/vdfs/address.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 拼接规则：单级 / 多级 / 两边多余分隔符 / 无父可接
#[test]
fn join_addr_normalizes_both_sides() {
    assert_eq!(join_addr("@root", ""), "@root");
    assert_eq!(join_addr("@root/", "/"), "@root");
    assert_eq!(
        join_addr("@root/session", "abc/MEMORY.md"),
        "@root/session/abc/MEMORY.md"
    );
    assert_eq!(join_addr("@root/session/", "/abc/"), "@root/session/abc");
    assert_eq!(join_addr("", "session/x"), "session/x");
    assert_eq!(join_addr("  ", "session/x"), "session/x");
}

/// 父地址首部的 `/` 是绝对锚点，必须被子地址带走（D-172）
///
/// 折叠掉它，从 `/docs` 出发的一次遍历就交出不带锚点的相对写法——绝对子树被读成
/// 了工作目录下的影子树，而且每一步都「成功」。
#[test]
fn join_addr_carries_the_absolute_anchor() {
    assert_eq!(join_addr("/", "docs"), "/docs");
    assert_eq!(join_addr("/docs", "a.md"), "/docs/a.md");
    assert_eq!(join_addr("/docs/", "a.md"), "/docs/a.md");
    assert_eq!(join_addr("/", ""), "/");
    assert_eq!(join_addr("//srv/share", "a.md"), "//srv/share/a.md");
    assert_eq!(join_addr("D:/tmp", "a.txt"), "D:/tmp/a.txt");
}
