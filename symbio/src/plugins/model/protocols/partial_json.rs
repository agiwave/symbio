//! 未结束 JSON 行的增量提取器（model 插件私有，四个协议共用）。
//!
//! ## 它解决什么
//!
//! SSE 的一行在换行到达之前是**半截 JSON**，`serde_json` 解不了。但为了首字延迟，
//! 我们希望「正文还在增长」这件事立刻反映到前端：不等换行，先把 `delta.content`
//! 这类**字符串字段**里已经确定的部分吐出去。
//!
//! 历史上这件事由 core 内置的启发式解析器代劳——在整行里搜 `"content":"` 之类的
//! 字面量。那套写法把协议字段名泄漏进了 core（加协议要改 core）、与协议解析器各
//! 自实现一遍 JSON 转义（两套规则 → 完整行按前缀截断时**吃字**）、且每收到一块就
//! 把整行重扫一遍（O(行长²)）。
//!
//! 本模块把这件事搬回协议层：core 只按 `\n` 切行，**协议**用 [`JsonLineExtractor`]
//! 声明「这一行里哪个位置的字符串要增量吐」。
//!
//! ## 契约（与 `symbio_core::sse::SsePartialLineExtractor` 一致）
//!
//! - **只吃新增字节**：内部状态机逐字节推进，已处理过的不再看第二遍 ⇒ 总代价
//!   O(行长)，不再是「每来一块重扫整行」。
//! - **解码规则与 `serde_json` 对齐**：本模块自己实现 JSON 字符串转义解码
//!   （`\n` / `\uXXXX` / 代理对…）。这是硬要求——core 把已发出的长度记成前缀，
//!   完整行到达时按前缀截断，两边解码不一致就会重复或吃字。
//! - **绝不吐半截**：被切断的多字节字符、不完整的转义序列（`\` 结尾、`\u12`）一律
//!   留到下一块再产出。
//!
//! ## 与完整行路径的分工
//!
//! 增量路径**不产出** `Finish` / `Usage` / `Error` / `ResponseId`：半截 JSON 里
//! 这些字段的值不可信。它们只走完整行的 `SseLineParser::parse_line`。
//! 因此本模块只回 `ContentDelta` / `ReasoningDelta` / `ToolCallDelta`。

use crate::symbio_core::ModelProtocolEvent;
use crate::symbio_core::SsePartialLineExtractor;

// ============ 协议侧实现的钩子 ============

/// 字符串值在 JSON 中的位置。
///
/// `keys` 是从根到该值的**对象键路径**（数组下标不占位）：例如
/// `choices[0].delta.content` → `["choices", "delta", "content"]`。
/// `array_index` 是**最内层**包住它的数组的元素下标（不在数组内则为 `None`）——
/// 工具调用参数要靠它区分是第几个调用。
pub struct FieldPath<'a> {
    keys: Vec<&'a str>,
    /// 最内层数组的元素下标
    pub array_index: Option<usize>,
}

impl FieldPath<'_> {
    /// 键路径是否**恰好**等于 `expected`。
    ///
    /// 刻意用「全等」而不是「包含 / 后缀」：错配的代价是把别处的文本当增量吐出去
    /// （前端多出内容，且完整行按前缀截断时会把正文吃掉）；不匹配的代价只是失去
    /// 增量、退回「等换行」这一原有行为。**宁可漏，不可错。**
    pub fn is(&self, expected: &[&str]) -> bool {
        self.keys.len() == expected.len() && self.keys.iter().zip(expected).all(|(a, b)| a == b)
    }

    /// 键路径（供测试断言）。
    #[cfg(test)]
    pub fn keys(&self) -> &[&str] {
        &self.keys
    }
}

/// 协议对某个字符串值的处理方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StrAction {
    /// 不关心
    Skip,
    /// 提取为事件：后续 [`PartialJsonSink::text`] 会逐段回调
    Emit,
    /// 只观察取值：后续 `text` 同样逐段回调，由实现方自行累计
    /// （用于 `type` 这类**判别字段**——它决定后面那些字段算内容还是推理）
    Observe,
}

/// 协议侧的判定钩子：本模块负责「在 JSON 的什么位置」，实现方负责「那是什么」。
pub trait PartialJsonSink: Send {
    /// 一个字符串值开始。返回 [`StrAction::Emit`] / [`StrAction::Observe`] 才会
    /// 收到后续的 [`Self::text`] 回调。
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction;

    /// 当前字符串的增量（**已按 JSON 转义规则解码**），把事件追加到 `out`。
    fn text(&mut self, text: &str, out: &mut Vec<ModelProtocolEvent>);

    /// 一个标量（数字 / `true` / `false` / `null`）结束，`raw` 是原文。
    ///
    /// 用来读 `output_index` / `index` 这类**决定事件归属**的数字：它们出现在目标
    /// 字符串之前，实现方在 `begin_string` 时据此判断索引是否已知——未知就放弃本
    /// 次增量（退回等换行），而不是用错索引产出事件。
    fn scalar(&mut self, path: &FieldPath<'_>, raw: &str) {
        let _ = (path, raw);
    }
}

// ============ 扫描器 ============

/// 扫描位置。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 尚未定位到 JSON 起点（`data: ` 前缀、Gemini 的 `,` 包裹、`[DONE]`…）
    Seek,
    /// 期待对象的键
    Key,
    /// 键已读完，等 `:`
    Colon,
    /// 期待值
    Value,
    /// 字符串中
    Str,
    /// 标量中（数字 / 字面量）
    Scalar,
    /// 值已结束，等 `,` 或闭合
    After,
}

/// 字符串内的转义状态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Esc {
    /// 不在转义中
    None,
    /// 刚读到 `\`，等转义字符
    Start,
    /// 正在读 `\uXXXX` 的十六进制位
    Hex { n: usize, v: u16 },
}

/// 容器种类
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Obj,
    Arr,
}

/// 一层容器
struct Frame {
    kind: FrameKind,
    /// 该容器作为「某个对象的成员」时的键；根容器与数组元素为 `None`
    key: Option<String>,
    /// 数组：已见元素个数（= 当前元素下标）
    count: usize,
}

/// 逐字节推进的 JSON 结构扫描 + 目标字符串增量提取。
pub struct JsonLineExtractor<S: PartialJsonSink> {
    sink: S,
    mode: Mode,
    /// 容器栈；`stack[0]` 是根容器
    stack: Vec<Frame>,
    /// 当前对象成员的键（值开始前有效）
    key: String,
    /// 正在读的字符串是「键」还是「值」
    reading_key: bool,
    /// 转义状态
    esc: Esc,
    /// 高代理项已就位、等低代理项（配对前**不产出**，否则会吐半个 emoji）
    high_surrogate: Option<u16>,
    /// 已解码但尚未交付的文本（复用缓冲）
    buf: String,
    /// 标量原文（复用缓冲）
    scalar_buf: String,
    /// 当前字符串的处置
    action: StrAction,
}

impl<S: PartialJsonSink> JsonLineExtractor<S> {
    /// 用协议侧的判定钩子构造一个提取器。
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            mode: Mode::Seek,
            stack: Vec::new(),
            key: String::new(),
            reading_key: false,
            esc: Esc::None,
            high_surrogate: None,
            buf: String::new(),
            scalar_buf: String::new(),
            action: StrAction::Skip,
        }
    }

    // ---- 结构 ----

    fn open(&mut self, kind: FrameKind) {
        // 只有「对象的成员」才有键；数组元素与根没有
        let key = match self.stack.last() {
            None => None,
            Some(f) if f.kind == FrameKind::Arr => None,
            Some(_) => Some(self.key.clone()),
        };
        self.stack.push(Frame {
            kind,
            key,
            count: 0,
        });
    }

    fn close(&mut self) {
        self.stack.pop();
        self.mode = Mode::After;
    }

    /// 当前值所处的路径：各层容器键（数组元素不占位）+ 当前成员键。
    ///
    /// 末尾的成员键**只在值确实是「对象的成员」时才追加**：数组元素没有键，若照抄
    /// 上一个成员的键，路径会多出一段陈旧键名（`a[0]` 被读成 `["a","a"]`）。
    ///
    /// 写成关联函数而不是 `&self` 方法：调用点要同时可变借用 `self.sink`，
    /// 借整个 `self` 的方法会让借用检查器无法拆分字段。
    fn path_of<'a>(stack: &'a [Frame], key: &'a str) -> FieldPath<'a> {
        let mut keys: Vec<&'a str> = Vec::with_capacity(stack.len() + 1);
        for f in stack {
            if let Some(k) = f.key.as_deref() {
                keys.push(k);
            }
        }
        if stack.last().is_some_and(|f| f.kind == FrameKind::Obj) {
            keys.push(key);
        }
        FieldPath {
            keys,
            array_index: stack
                .iter()
                .rev()
                .find(|f| f.kind == FrameKind::Arr)
                .map(|f| f.count),
        }
    }

    // ---- 字符串 ----

    fn start_string(&mut self, is_key: bool) {
        self.reading_key = is_key;
        self.buf.clear();
        self.esc = Esc::None;
        self.high_surrogate = None;
        self.mode = Mode::Str;
        self.action = StrAction::Skip;
        if !is_key {
            let path = Self::path_of(&self.stack, &self.key);
            self.action = self.sink.begin_string(&path);
        }
    }

    /// 字符串结束：键则落到 `self.key`，值则交付尾巴。
    fn end_string(&mut self, out: &mut Vec<ModelProtocolEvent>) {
        self.settle_surrogate();
        if self.reading_key {
            self.key.clear();
            self.key.push_str(&self.buf);
            self.buf.clear();
            self.action = StrAction::Skip;
            self.mode = Mode::Colon;
        } else {
            self.flush_text(out);
            self.action = StrAction::Skip;
            self.mode = Mode::After;
        }
        self.reading_key = false;
    }

    /// 把已解码的文本交给 sink（值才交；键在攒全文）。
    fn flush_text(&mut self, out: &mut Vec<ModelProtocolEvent>) {
        if self.reading_key || self.buf.is_empty() {
            return;
        }
        if self.action != StrAction::Skip {
            let buf = std::mem::take(&mut self.buf);
            self.sink.text(&buf, out);
            self.buf = buf;
            self.buf.clear();
        } else {
            self.buf.clear();
        }
    }

    /// 追加一段普通（非转义）字符。
    fn push_literal(&mut self, run: &str) {
        self.settle_surrogate();
        self.buf.push_str(run);
    }

    /// 高代理项悬空却等不到低代理项时，补一个替换字符——
    /// 与 `serde_json` 的「拒绝」不同，这里选择**不吞掉**后续内容。
    fn settle_surrogate(&mut self) {
        if self.high_surrogate.take().is_some() {
            self.buf.push('\u{FFFD}');
        }
    }

    fn push_u16(&mut self, v: u16) {
        if let Some(hi) = self.high_surrogate.take() {
            if (0xDC00..=0xDFFF).contains(&v) {
                let c = 0x1_0000 + (((hi as u32) - 0xD800) << 10) + ((v as u32) - 0xDC00);
                if let Some(ch) = char::from_u32(c) {
                    self.buf.push(ch);
                    return;
                }
            }
            self.buf.push('\u{FFFD}');
        }
        if (0xD800..=0xDBFF).contains(&v) {
            self.high_surrogate = Some(v);
        } else if (0xDC00..=0xDFFF).contains(&v) {
            self.buf.push('\u{FFFD}');
        } else if let Some(ch) = char::from_u32(v as u32) {
            self.buf.push(ch);
        }
    }

    /// 消费一个转义字节，返回实际前进的字节数。
    fn consume_escape(&mut self, s: &str, i: usize) -> usize {
        let c = s.as_bytes()[i];
        match self.esc {
            Esc::None => {
                self.esc = Esc::Start;
                1
            }
            Esc::Start => {
                match c {
                    b'u' => self.esc = Esc::Hex { n: 0, v: 0 },
                    b'"' => {
                        self.settle_surrogate();
                        self.buf.push('"');
                        self.esc = Esc::None;
                    }
                    b'\\' => {
                        self.settle_surrogate();
                        self.buf.push('\\');
                        self.esc = Esc::None;
                    }
                    b'/' => {
                        self.settle_surrogate();
                        self.buf.push('/');
                        self.esc = Esc::None;
                    }
                    b'b' => {
                        self.settle_surrogate();
                        self.buf.push('\u{8}');
                        self.esc = Esc::None;
                    }
                    b'f' => {
                        self.settle_surrogate();
                        self.buf.push('\u{c}');
                        self.esc = Esc::None;
                    }
                    b'n' => {
                        self.settle_surrogate();
                        self.buf.push('\n');
                        self.esc = Esc::None;
                    }
                    b'r' => {
                        self.settle_surrogate();
                        self.buf.push('\r');
                        self.esc = Esc::None;
                    }
                    b't' => {
                        self.settle_surrogate();
                        self.buf.push('\t');
                        self.esc = Esc::None;
                    }
                    _ => {
                        // 非法转义（`serde_json` 会整体拒绝该行）。宽松处理：原样
                        // 输出，避免因为一个坏字节把后面全部内容吞掉。
                        self.settle_surrogate();
                        let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
                        self.buf.push(ch);
                        self.esc = Esc::None;
                        return ch.len_utf8();
                    }
                }
                1
            }
            Esc::Hex { n, v } => {
                let Some(d) = (c as char).to_digit(16).map(|d| d as u16) else {
                    // 非法十六进制位：放弃该转义，**不消费**当前字节（下一轮当普通
                    // 字符处理，保证一定前进）
                    self.esc = Esc::None;
                    self.settle_surrogate();
                    return 0;
                };
                let nv = v * 16 + d;
                if n + 1 == 4 {
                    self.esc = Esc::None;
                    self.push_u16(nv);
                } else {
                    self.esc = Esc::Hex { n: n + 1, v: nv };
                }
                1
            }
        }
    }

    // ---- 标量 ----

    fn finish_scalar(&mut self) {
        if self.scalar_buf.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.scalar_buf);
        let path = Self::path_of(&self.stack, &self.key);
        self.sink.scalar(&path, &raw);
        self.scalar_buf = raw;
        self.scalar_buf.clear();
    }

    // ---- 主循环 ----

    fn feed(&mut self, s: &str, out: &mut Vec<ModelProtocolEvent>) {
        let b = s.as_bytes();
        let mut i = 0usize;
        while i < b.len() {
            match self.mode {
                Mode::Seek => {
                    match b[i] {
                        b'{' => {
                            self.open(FrameKind::Obj);
                            self.mode = Mode::Key;
                        }
                        b'[' => {
                            self.open(FrameKind::Arr);
                            self.mode = Mode::Value;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                Mode::Key => match b[i] {
                    b'"' => {
                        self.start_string(true);
                        i += 1;
                    }
                    b'}' => {
                        self.close();
                        i += 1;
                    }
                    _ => i += 1,
                },
                Mode::Colon => {
                    if b[i] == b':' {
                        self.mode = Mode::Value;
                    }
                    i += 1;
                }
                Mode::Value => match b[i] {
                    b'"' => {
                        self.start_string(false);
                        i += 1;
                    }
                    b'{' => {
                        self.open(FrameKind::Obj);
                        self.mode = Mode::Key;
                        i += 1;
                    }
                    b'[' => {
                        self.open(FrameKind::Arr);
                        self.mode = Mode::Value;
                        i += 1;
                    }
                    b'}' | b']' => {
                        self.close();
                        i += 1;
                    }
                    b't' | b'f' | b'n' | b'-' | b'0'..=b'9' => {
                        self.scalar_buf.clear();
                        self.mode = Mode::Scalar;
                    }
                    _ => i += 1,
                },
                Mode::Scalar => {
                    match b[i] {
                        // 分隔符不消费：交给 After / close 分支处理
                        b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n' => {
                            self.finish_scalar();
                            self.mode = Mode::After;
                        }
                        _ => {
                            let ch = s[i..].chars().next().unwrap_or('\u{FFFD}');
                            self.scalar_buf.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                Mode::After => match b[i] {
                    b',' => {
                        if let Some(f) = self.stack.last_mut() {
                            match f.kind {
                                FrameKind::Obj => self.mode = Mode::Key,
                                FrameKind::Arr => {
                                    f.count += 1;
                                    self.mode = Mode::Value;
                                }
                            }
                        }
                        i += 1;
                    }
                    b'}' | b']' => {
                        self.close();
                        i += 1;
                    }
                    _ => i += 1,
                },
                Mode::Str => {
                    if self.esc != Esc::None {
                        i += self.consume_escape(s, i);
                        continue;
                    }
                    let rest = &b[i..];
                    match rest.iter().position(|&x| x == b'"' || x == b'\\') {
                        Some(0) if rest[0] == b'"' => {
                            self.end_string(out);
                            i += 1;
                        }
                        Some(0) => {
                            self.esc = Esc::Start;
                            i += 1;
                        }
                        Some(p) => {
                            // `p` 落在 ASCII 字节上 ⇒ 一定是字符边界
                            self.push_literal(&s[i..i + p]);
                            i += p;
                        }
                        None => {
                            self.push_literal(&s[i..]);
                            i = b.len();
                        }
                    }
                }
            }
        }
        // 块末交付：长字符串不能等闭合才可见
        self.flush_text(out);
    }
}

impl<S: PartialJsonSink + 'static> SsePartialLineExtractor for JsonLineExtractor<S> {
    fn push(&mut self, bytes: &str, out: &mut Vec<ModelProtocolEvent>) {
        self.feed(bytes, out);
    }
}

// ============ 测试辅助（四个协议的测试共用） ============

/// 把一行按固定块长喂给提取器，返回累计产出的事件。
///
/// 只在字符边界切分——core 的 `utf8_chunk` 保证喂进来的永远是合法 UTF-8。
#[cfg(test)]
pub(crate) fn feed_chunked(
    ext: &mut dyn SsePartialLineExtractor,
    line: &str,
    size: usize,
) -> Vec<ModelProtocolEvent> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let mut end = (start + size).min(line.len());
        while end < line.len() && !line.is_char_boundary(end) {
            end += 1;
        }
        ext.push(&line[start..end], &mut out);
        start = end;
    }
    out
}

/// 把事件序列里的文本（内容 / 推理 / 工具参数）拼起来。
#[cfg(test)]
pub(crate) fn text_of(evs: &[ModelProtocolEvent]) -> String {
    let mut s = String::new();
    for e in evs {
        match e {
            ModelProtocolEvent::ContentDelta(t) | ModelProtocolEvent::ReasoningDelta(t) => {
                s.push_str(t)
            }
            ModelProtocolEvent::ToolCallDelta(_, _, _, Some(a)) => s.push_str(a),
            _ => {}
        }
    }
    s
}

/// 提取器产出的工具调用参数，按下标**合并**成整串。
///
/// 增量路径会把一段参数拆成多条事件（每块一条），而完整行路径只给一条——
/// 比对前必须先合并，否则比的是「切了几块」而不是内容。
#[cfg(test)]
pub(crate) fn tool_args_of(evs: &[ModelProtocolEvent]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for e in evs {
        if let ModelProtocolEvent::ToolCallDelta(i, _, _, Some(a)) = e {
            match out.last_mut() {
                Some((last, s)) if last == i => s.push_str(a),
                _ => out.push((*i, a.clone())),
            }
        }
    }
    out
}

/// **一致性不变量**：任意块边界下增量提取出的文本，必须等于整行解析出的文本。
///
/// 这是本模块存在的全部理由——core 按前缀截断去重，两边对不上就会重复或吃字。
#[cfg(test)]
pub(crate) fn assert_partial_matches_full_line(
    p: &dyn crate::symbio_core::SseLineParser,
    line: &str,
) {
    let full = text_of(&p.parse_line(line));
    for chunk in 1..=line.len() {
        let mut ext = p.open_partial_line(line).expect("本协议应实现增量提取");
        let got = text_of(&feed_chunked(ext.as_mut(), line, chunk));
        assert_eq!(
            got, full,
            "chunk={chunk} 时增量文本与完整行不一致\nline={line}"
        );
    }
}

/// 同上，但比对工具调用参数（下标 + 文本）。
#[cfg(test)]
pub(crate) fn assert_tool_args_match_full_line(
    p: &dyn crate::symbio_core::SseLineParser,
    line: &str,
) {
    let full = tool_args_of(&p.parse_line(line));
    for chunk in 1..=line.len() {
        let mut ext = p.open_partial_line(line).expect("本协议应实现增量提取");
        let got = tool_args_of(&feed_chunked(ext.as_mut(), line, chunk));
        assert_eq!(
            got, full,
            "chunk={chunk} 时工具参数与完整行不一致\nline={line}"
        );
    }
}

#[cfg(test)]
#[path = "partial_json.test.rs"]
mod tests;
