//! 字符串安全截断工具
//!
//! ## 为什么需要
//!
//! Rust 的 `&s[..n]` 按**字节**切片，若 `n` 落在多字节字符（CJK、emoji 等）
//! 内部会直接 panic：`byte index N is not a char boundary`。
//!
//! 日志摘要 / 错误文案截断几乎都写成 `if s.len() > 200 { &s[..200] }`，
//! 在纯 ASCII 场景下相安无事，一旦内容含中文就会在**运行时**炸出 panic——
//! 而且往往发生在工具执行链路里，表现为整轮会话异常终止
//! （真实事故：一次工具调用传入中文参数 → `args_summary` 切片越界 →
//! `tokio-rt-worker` panic → ChatLoop 任务 join 失败）。
//!
//! 因此所有"按显示长度截断"的场景都应走本模块，而不是裸切片。

//! ## 本域的第二块：字符口径截断
//!
//! [`merge_kernel`] 装的是按**字符数**截断——与本域两个字节口径函数同族（都在解决
//! 「中文一字多字节，按字节切会切坏」这一件事）。它有两个不同 owner 的生产消费者
//! （memory 的情景编码、skill 的技能编译），因此留在共享文本域。
//!
//! 同族的**巩固合并算法**（分句 / 合并 + 保真度）只有一个消费者（memory 的巩固写口），
//! 按共享库准入的第一条（≥2 个不同 owner）**一律私有**，已随写口搬进
//! `symbio-plugin-memory` 的 `src/consolidate/merge.rs`，本域不再留副本。
//!
//! 第三块是 [`tokenizer`]：预算的计量与头尾切分。它原住门面（`plugin-api`），
//! 但那里只该装词、宏与转发——纯机制归共享库（`D-404`），门面留一行转发。

use std::borrow::Cow;

pub mod merge_kernel;
/// Token 计量与头尾切分（预算的计量基础；`D-404`）。
pub mod tokenizer;

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
