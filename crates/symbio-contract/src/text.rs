//! 字节口径的**纯文本截断**（真实事故驱动：中文参数按字节切片 → `tokio-rt-worker`
//! panic → ChatLoop 任务 join 失败）。纯 std、零状态零 IO、内核不读、`local` / `mcp` /
//! `web` / provider-memory 多 owner 共享 ⇒ 规则 B（D-486）。字符口径的
//! [`crate::merge_kernel::truncate_chars`] 同族同批迁入。
//!
//! 「按显示长度截断」一律走本模块，不写裸切片：Rust 的 `&s[..n]` 按**字节**切片，
//! `n` 落在多字节字符（CJK、emoji）内部会直接 panic。tokenizer（token 计量与头尾
//! 切分）是有预算语义的宿主机制（`D-404`），留服务层。

use std::borrow::Cow;

/// 按字节上限安全截断：返回不超过 `max_bytes` 的最长合法前缀。
///
/// 若发生截断（原串更长），返回值末尾带 `suffix`（通常是 `…`）；
/// 未截断时原样返回。`max_bytes` 为 0 时返回空串（不 panic）。
pub fn text_truncate_bytes<'a>(s: &'a str, max_bytes: usize, suffix: &str) -> Cow<'a, str> {
    if s.len() <= max_bytes {
        return Cow::Borrowed(s);
    }
    let mut end = max_bytes;
    // 回退到最近的合法字符边界（最多回退 3 字节，UTF-8 单字符上限）
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(format!("{}{suffix}", &s[..end]))
}

/// 按字节上限安全截断，**不带**省略号后缀：返回合法前缀切片。
///
/// 用于"必须仍是原串子串"的场景（如按字节预算分块发送）。
pub fn text_floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests;
