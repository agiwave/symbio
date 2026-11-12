
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
