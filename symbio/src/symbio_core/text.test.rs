//! `symbio/src/symbio_core/text.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 中文字符跨越字节切片点时不得 panic（真实事故场景）：
/// `args_summary(&json!({...中文...}), 200)` 在 byte 200
/// 落在 '的'（bytes 199..202）内部 → `tokio-rt-worker` panic。
#[test]
fn truncate_does_not_panic_on_multibyte_boundary() {
    let s = "参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。".repeat(10);
    for n in 0..s.len() {
        // 任何字节上限都必须安全返回，绝不 panic
        let out = truncate_bytes(&s, n, "…");
        assert!(out.len() <= n.max(0) + 4, "n={n} 输出超长：{}", out.len());
    }
}

#[test]
fn truncate_shorter_than_limit_returns_original() {
    assert_eq!(truncate_bytes("hello", 10, "…"), "hello");
    assert_eq!(truncate_bytes("hello", 5, "…"), "hello");
}

#[test]
fn truncate_exact_boundary_no_suffix_when_equal_len() {
    // len == max_bytes 视为未截断（避免把刚好合规的内容也打省略号）
    assert_eq!(truncate_bytes("héllo", 6, "…"), "héllo");
}

#[test]
fn truncate_ascii_at_limit_appends_suffix() {
    assert_eq!(truncate_bytes("abcdef", 3, "..."), "abc...");
}

#[test]
fn floor_char_boundary_never_splits_char() {
    let s = "中文abc";
    for n in 0..=s.len() {
        let b = floor_char_boundary(s, n);
        assert!(s.is_char_boundary(b), "n={n} 得到非法边界 {b}");
        assert!(b <= n);
    }
    assert_eq!(floor_char_boundary(s, 1), 0); // '中' 占 3 字节 → 回退到 0
    assert_eq!(floor_char_boundary(s, 2), 0);
    assert_eq!(floor_char_boundary(s, 3), 3);
    assert_eq!(floor_char_boundary(s, 999), s.len());
}

#[test]
fn zero_budget_is_safe() {
    assert_eq!(truncate_bytes("中文", 0, "…"), "…");
    assert_eq!(floor_char_boundary("中文", 0), 0);
}
