//! `sse.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。

use super::*;

/// 完整 UTF-8：整段消费。
#[test]
fn utf8_chunk_consumes_complete_text() {
    let buf = "data: hello\n".as_bytes();
    assert_eq!(utf8_chunk(buf, 0), ("data: hello\n", 12));
}

/// 从中间开始（已消费前缀）：只给新增部分。
#[test]
fn utf8_chunk_resumes_from_offset() {
    let buf = "data: hello".as_bytes();
    assert_eq!(utf8_chunk(buf, 6), ("hello", 11));
}

/// 多字节字符被切断 ⇒ **不消费**尾部残字节，留给下一次。
///
/// 这是 SSE 分块下的常态（TCP 不保证按字符边界切），必须由契约保证：
/// 否则两块各替换出一个 U+FFFD，而完整行给出的是真字符，按前缀截断就会吃字。
#[test]
fn utf8_chunk_holds_back_split_multibyte_char() {
    let full = "data: 你好".as_bytes();
    // 「好」占 3 字节，切掉最后一字节
    let cut = full.len() - 1;
    let (s, consumed) = utf8_chunk(&full[..cut], 0);
    assert_eq!(s, "data: 你");
    assert_eq!(consumed, 9, "「好」的残字节不得被消费");

    // 补齐后从上次消费处再取，拿到剩下的完整字符
    let (s, consumed) = utf8_chunk(full, consumed);
    assert_eq!(s, "好");
    assert_eq!(consumed, full.len());
}

/// 从字符中间起（`from` 落在多字节字符内部）⇒ 一个字节都不消费。
///
/// 调用方只在边界处推进，理论上不会出现；这条用例把「万一」钉死：
/// 宁可这一轮什么都不喂，也不能喂出替换字符。
#[test]
fn utf8_chunk_from_inside_multibyte_consumes_nothing() {
    let buf = "你".as_bytes();
    let (s, consumed) = utf8_chunk(buf, 1);
    assert_eq!(s, "");
    assert_eq!(consumed, 1, "位置不得前进");
}

/// 非法字节序列（不是「被切断」而是真的坏）：只消费到坏字节之前。
#[test]
fn utf8_chunk_stops_before_invalid_bytes() {
    let buf = [b'a', b'b', 0xFF, b'c'];
    let (s, consumed) = utf8_chunk(&buf, 0);
    assert_eq!(s, "ab");
    assert_eq!(consumed, 2);
}
