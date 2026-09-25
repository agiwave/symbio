//! SSE 行解析契约：**core 只按行切分，协议层负责「这一行是什么」**。
//!
//! ## 为什么需要这两个 trait
//!
//! `parse_sse_stream` 要处理两种输入：
//! 1. **完整行**（已收到换行）——交给协议解析器，得到事件序列；
//! 2. **未结束的行**（字节在积攒中）——为了首字延迟，希望**边收边吐**。
//!
//! 第 2 种情况不能拿 `serde_json` 去解（JSON 被截断了）。历史上 core 为此内置了
//! 一个**启发式解析器**（`try_parse_partial_sse_line`），里面硬编码了
//! `"content":"` / `"reasoning_content":"` / `"partial_json":"` / `"text":"` /
//! `"arguments":"` 五个字段名——**协议知识泄漏进了 core**。后果有三：
//!
//! - **加协议要改 core**：新协议（如 Gemini 的 `parts[].text`）能不能增量提取，
//!   取决于 core 里那张表有没有它的字段；
//! - **两条路径两套转义**：core 的 `unescape_partial` 与协议解析器的
//!   `serde_json` 各自实现一遍 JSON 字符串转义，二者对 `\uXXXX` 的处理不同，
//!   于是「已发送前缀长度」记的是 A 的长度、完整行给的是 B 的文本——
//!   完整行到达时的按前缀截断会**吃掉字符**；
//! - **每次都重扫整行**：缓冲区每增长一次就把整行重新解析一遍，单行极长时是 O(n²)。
//!
//! ## 收口后的分工
//!
//! - [`SseLineParser::parse_line`]：完整行 → 事件（协议实现，等价于历史上的闭包）；
//! - [`SseLineParser::open_partial_line`]：为未结束的行开一个**有状态**的
//!   [`SsePartialLineExtractor`]，由协议决定「这一行值不值得增量提取」「取哪个字段」。
//!   core 只负责：每收到新字节就 `push` 一次，把返回的增量原样转发。
//!
//! 增量提取器是**有状态**的（只对新增字节做功），因此总代价是 O(输入长度)，
//! 不再是 O(行长的平方)；字段名与转义规则全部留在协议层，core 不再认识任何
//! 协议细节。

use super::model_provider::ModelProtocolEvent;

/// SSE 行解析契约（协议层实现）。
pub trait SseLineParser: Send + Sync {
    /// 完整行（已含换行，调用方已 `trim`）→ 事件序列。
    fn parse_line(&self, line: &str) -> Vec<ModelProtocolEvent>;

    /// 为一条**尚未结束**的行开一个增量提取器。
    ///
    /// `head` 是这一行到目前为止已收到的全部字节（UTF-8 边界已对齐）。
    ///
    /// 返回 `None` ⇒ **本行不做增量提取**：core 会一直等到换行再走
    /// [`Self::parse_line`]。这是合法且正确的降级（只是首字延迟变大），
    /// 因此不实现增量提取的协议无需任何额外代码——默认实现就是 `None`。
    ///
    /// 实现方应当在**行首就能判断**（例如「这个事件类型携带全量文本，不能当增量」），
    /// 因为 core 只会调用它一次：返回 `None` 后本行不再重试。
    fn open_partial_line(&self, head: &str) -> Option<Box<dyn SsePartialLineExtractor>> {
        let _ = head;
        None
    }
}

/// 单行增量提取器：逐段吃进原始字节，吐出**本次新增**的增量事件。
///
/// 契约（实现方必须保证，core 依赖它）：
/// - `push` 的入参是**自上次 push 之后新增的字节**，不重复、不遗漏；
/// - 产出的增量必须与 [`SseLineParser::parse_line`] 对**同一完整行**给出的文本
///   **逐字节一致地拼接**——core 会把已发出的长度记为前缀，完整行到达时按该
///   前缀截断。二者不一致就会重复或丢字。
///
/// 事件写进 `out`（调用方复用同一个 `Vec`，避免每块一次分配）：一次 `push`
/// 可能产出**多个**事件——同一块里可能同时结束一个字段、开始下一个（例如
/// 一个 SSE 行里 `content` 收尾紧接 `tool_calls[0].function.arguments` 开头）。
pub trait SsePartialLineExtractor: Send {
    /// 追加新收到的字节，把本次新增的事件追加到 `out`。
    fn push(&mut self, bytes: &str, out: &mut Vec<ModelProtocolEvent>);
}

/// 从字节缓冲的 `from` 起取一段**UTF-8 边界对齐**的字符串。
///
/// 返回 `(切片, 已消费到的位置)`：尾部若有多字节字符被切断，**不消费**它
/// （`已消费位置` 停在字符起始处），留给下一次 `push`。
///
/// 为什么必须对齐：SSE 分块由 TCP 决定，一个中文字符横跨两块是常态。
/// 直接 `from_utf8_lossy` 会把两块各替换成一个 U+FFFD，而完整行到达时给出的是
/// 真字符——已发送长度按替换字符算，截断就会吃掉正文。
pub(crate) fn utf8_chunk(buf: &[u8], from: usize) -> (&str, usize) {
    let rest = &buf[from..];
    let n = match std::str::from_utf8(rest) {
        Ok(_) => rest.len(),
        Err(e) => e.valid_up_to(),
    };
    // 上面已确认 `rest[..n]` 是合法 UTF-8，`unwrap_or("")` 只是不引入 unsafe 的兜底。
    (std::str::from_utf8(&rest[..n]).unwrap_or(""), from + n)
}

#[cfg(test)]
#[path = "sse.test.rs"]
mod tests;
