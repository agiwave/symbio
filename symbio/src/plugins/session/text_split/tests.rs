//! `text_split` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `text_split.rs` 只保留生产代码，测试全部放本文件。

use super::super::tokenizer::HeuristicTokenizer;
use super::*;

/// 测试专用：确定性计量器（不读进程级校准状态，结果可复现）。
fn split(text: &str, head_budget: usize, tail_budget: usize) -> HeadTailSplit {
    split_head_tail(text, head_budget, tail_budget, &HeuristicTokenizer)
}

/// 短文本整体放进 head：逐行带回换行，tail 为空。
#[test]
fn short_text_kept_whole_in_head() {
    let s = split("a\nb\nc", 100, 100);
    assert_eq!(s.head, "a\nb\nc\n");
    assert_eq!(s.tail, "");
}

/// 单行超预算：head 以"行首截断 + `…`"收尾，不会整行放行；tail 预算充足时
/// 该行仍整行保留在尾部（与原 L0 行为一致——head 兜底只保证不整行放行，
/// 不代表该行从 tail 中消失）。
#[test]
fn single_long_line_head_truncated() {
    let line = "x".repeat(50);
    let s = split(&line, 5, 100);
    assert_eq!(s.head, format!("{}…\n", "x".repeat(10)), "字符上限 = 5×2");
    assert_eq!(
        s.tail, line,
        "50 字符 ≈ 15 token ≤ tail 预算 100，整行进 tail"
    );
}

/// 尾行超预算：tail 以 `…` 开头且保留行尾字符。
#[test]
fn tail_line_truncation_keeps_line_end() {
    // 计量（HeuristicTokenizer）：head_line ≈ 4 token（3+换行），
    // y 行 ≈ 15 token；head 预算 10 → 首行可入、y 行必超。
    let text = format!("head_line\n{}", "y".repeat(50));
    let s = split(&text, 10, 5);
    assert_eq!(s.head, "head_line\n");
    assert_eq!(
        s.tail,
        format!("…{}", "y".repeat(10)),
        "尾行保行尾 5×2 字符"
    );
}

/// 头尾不重叠：tail 从 head 取走的部分之后开始。
#[test]
fn head_and_tail_do_not_overlap() {
    let text = "l1\nl2\nl3\nl4\nl5";
    let s = split(text, 4, 4);
    let head_lines: Vec<&str> = s.head.lines().collect();
    let tail_lines: Vec<&str> = s.tail.lines().collect();
    assert_eq!(head_lines, vec!["l1", "l2"]);
    assert_eq!(tail_lines, vec!["l4", "l5"]);
    assert!(head_lines.iter().all(|h| !tail_lines.contains(h)));
}

/// 字符兜底按**字符**而非字节截取（多字节 CJK 安全）。
#[test]
fn char_truncation_is_multibyte_safe() {
    let cjk = "你好世界";
    assert_eq!(truncate_head_chars(cjk, 1), "你好");
    assert_eq!(truncate_tail_chars(cjk, 1), "世界");
}

/// 条件截断：未超限原样返回（无省略号），超限才截断并加省略号。
#[test]
fn conditional_truncation_only_when_over() {
    assert_eq!(truncate_head_chars_if_over("hello", 10), "hello");
    let t = truncate_head_chars_if_over(&"z".repeat(30), 10);
    assert_eq!(t, format!("{}…", "z".repeat(20)));
}
