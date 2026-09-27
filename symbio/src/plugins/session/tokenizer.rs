//! Token 估算与头尾切分：上下文预算的计量基础与超预算的文本切分机制
//!
//! ## 计量与切分为什么住在同一个文件
//!
//! [`split_head_tail`] 的签名里就有 `&dyn Tokenizer`——切分由 token 计量驱动，
//! 两者是同一件「预算怎么花」的两半；切分若另起一文件，就只能反向引用本文件的
//! 计量器，凭空多出一层。分工见 [`split_head_tail`] 上方的注释。
//!
//! ## 为什么需要独立模块
//!
//! 上下文裁剪（轮次窗口 / 工具结果窗口 / 自动摘要）的所有判据最终都要落到
//! "这段内容占多少 token"。此前 `plugins/model/compression.rs` 用
//! `text.len() / 4` 估算——`len()` 是 **UTF-8 字节数**，
//! 中文 1 字 = 3 字节被估成 0.75 token（实际约 1.0~1.5），
//! 低估约 2 倍，导致压缩触发时上下文早已超限。
//!
//! ## 设计取向
//!
//! - **零依赖、纯 Rust**：不引入任何 C/C++ 编译（项目铁律），也不引入 bpe 词表。
//! - **宁可高估**：高估只是早压缩一点；低估会直接招来 provider 的 400。
//! - **可替换**：[`Tokenizer`] 是 trait，未来可插真实 tokenizer（如 tiktoken-rs），
//!   调用方只依赖 trait，无需改动。
//! - **可校准**：[`CalibratedTokenizer`] 用 provider 返回的真实 `usage` 反馈
//!   做滑动校正，把启发式的系统偏差逐步拉回 1.0（对中文场景收益最大）。
//!
//! ## Phase sink
//!
//! 自 `symbio_core/tokenizer.rs` 下沉至 session 插件——E-② 后唯一消费者
//! （compression / chat_loop / tool_result_guard）全部位于 session 模块内，
//! 属单一模块私有设施，不再置于 core 共享层。

use std::sync::atomic::{AtomicU32, Ordering};

/// 采样阈值：超过该字符数时先采样再外推，避免超大输入阻塞请求
const SAMPLE_CHARS: usize = 32_768;

/// token 计数抽象
///
/// 体检结论（audit-4）：trait 收敛为单一方法 `count`——纯文本计量归本模块；
/// 消息级 / 工具级聚合（结构开销、ToolCall 参数、prompt 前缀等语义）
/// 由 compression.rs 的 estimate_* 系列负责，避免两处重复定义。
pub trait Tokenizer: Send + Sync {
    /// 估算单段文本的 token 数
    fn count(&self, text: &str) -> usize;
}

/// 字符类别权重（每字符的 token 数，放大 1000 倍存整数避免浮点）
mod weight {
    /// CJK 统一表意文字 / 假名 / 谚文 / 全角标点：约 1 token/字
    pub const CJK: u32 = 1000;
    /// 代码高信号字符：`{}()[]<>;=+/\|&*!?#$@~^\``
    pub const CODE: u32 = 500;
    /// 空白：约 4~6 字符 1 token
    pub const WHITESPACE: u32 = 150;
    /// 其余（ASCII 字母数字、常见标点）：约 3.5 字符 1 token
    pub const DEFAULT: u32 = 280;
}

/// 默认实现：按字符类别加权的启发式估算
///
/// **必须按 `chars()` 遍历，绝不能用 `len()`（字节数）**——这是本模块存在的原因。
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicTokenizer;

impl Tokenizer for HeuristicTokenizer {
    fn count(&self, text: &str) -> usize {
        let char_count = text.chars().count();
        // 超大输入：取前 SAMPLE_CHARS 个字符估算后按比例外推
        if char_count > SAMPLE_CHARS {
            let head: String = text.chars().take(SAMPLE_CHARS).collect();
            let sampled = weighted_sum(&head);
            return (sampled as f64 * (char_count as f64 / SAMPLE_CHARS as f64)) as usize / 1000
                + 1;
        }
        weighted_sum(text) / 1000 + 1
    }
}

/// 按字符类别累加权重（单位：1/1000 token）
fn weighted_sum(text: &str) -> usize {
    let mut total: usize = 0;
    for ch in text.chars() {
        total += weight_of(ch) as usize;
    }
    total
}

fn weight_of(ch: char) -> u32 {
    if is_cjk(ch) {
        weight::CJK
    } else if ch.is_whitespace() {
        weight::WHITESPACE
    } else if is_code_char(ch) {
        weight::CODE
    } else {
        weight::DEFAULT
    }
}

/// CJK 及全角区间判定（含中日韩统一表意文字、假名、谚文、全角/中文标点）
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{1100}'..='\u{11FF}'   // 韩文（谚文）字母
        | '\u{2E80}'..='\u{2EFF}' // CJK 部首补充
        | '\u{2F00}'..='\u{2FDF}' // 康熙部首
        | '\u{3000}'..='\u{303F}' // CJK 符号和标点（含全角空格）
        | '\u{3040}'..='\u{309F}' // 日文平假名
        | '\u{30A0}'..='\u{30FF}' // 日文片假名
        | '\u{3130}'..='\u{318F}' // 韩文兼容字母
        | '\u{31C0}'..='\u{31EF}' // CJK 笔画
        | '\u{31F0}'..='\u{31FF}' // 片假名语音扩展
        | '\u{3400}'..='\u{4DBF}' // CJK 扩展 A
        | '\u{4E00}'..='\u{9FFF}' // CJK 基本区
        | '\u{A960}'..='\u{A97F}' // 韩文（谚文）扩展 A
        | '\u{AC00}'..='\u{D7AF}' // 韩文（谚文）音节
        | '\u{F900}'..='\u{FAFF}' // CJK 兼容表意文字
        | '\u{FE30}'..='\u{FE4F}' // CJK 兼容形式
        | '\u{FF00}'..='\u{FF60}' // 全角 ASCII 变体
        | '\u{FFE0}'..='\u{FFE6}' // 全角符号
        | '\u{20000}'..='\u{2FA1F}' // CJK 扩展 B..F
    )
}

fn is_code_char(ch: char) -> bool {
    matches!(
        ch,
        '{' | '}'
            | '('
            | ')'
            | '['
            | ']'
            | '<'
            | '>'
            | ';'
            | '='
            | '+'
            | '/'
            | '\\'
            | '|'
            | '&'
            | '*'
            | '!'
            | '?'
            | '#'
            | '$'
            | '@'
            | '~'
            | '^'
            | '`'
            | '_'
            | '%'
    )
}

/// 用量校准器：用 provider 返回的真实用量反馈修正启发式的系统偏差
///
/// 存一个滑动平均比值 `actual / estimate`（放大 10000 倍）。
/// 初值 1.0；每次拿到真实用量就按 `alpha` 混合更新。
/// 这样即便启发式对某类文本（如中文、代码）系统性偏差，几轮之后也能收敛。
#[derive(Debug)]
pub struct CalibratedTokenizer {
    inner: HeuristicTokenizer,
    /// ratio × 10000
    ratio: AtomicU32,
}

impl Default for CalibratedTokenizer {
    fn default() -> Self {
        Self {
            inner: HeuristicTokenizer,
            ratio: AtomicU32::new(10_000),
        }
    }
}

impl CalibratedTokenizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 反馈一次真实用量：`actual` 为该次请求的真实 token 数，
    /// `estimated` 为**未经校准的原始启发式估算值**（[`Self::count_raw`]）。
    ///
    /// ⚠️ 不要把校准后的 `count()` 结果传进来：那会让观测比值依赖当前校准系数，
    /// 数学上收敛到 √(真实比值) 而非真实比值（自反馈偏差）。两者任一为 0 则忽略。
    pub fn feedback(&self, actual: usize, estimated: usize) {
        if actual == 0 || estimated == 0 {
            return;
        }
        let observed =
            ((actual as f64 / estimated as f64) * 10_000.0).clamp(2_000.0, 50_000.0) as u32;
        // alpha = 0.3 的指数滑动平均
        let cur = self.ratio.load(Ordering::Relaxed);
        let next = (cur as f64 * 0.7 + observed as f64 * 0.3) as u32;
        self.ratio.store(next.max(2_000), Ordering::Relaxed);
    }
}

impl Tokenizer for CalibratedTokenizer {
    fn count(&self, text: &str) -> usize {
        let base = self.inner.count(text);
        let r = self.ratio.load(Ordering::Relaxed) as f64 / 10_000.0;
        (base as f64 * r) as usize + 1
    }
}

impl CalibratedTokenizer {
    /// 原始启发式估算（不含校准系数）。
    ///
    /// 反馈 [`Self::feedback`] 时必须用这个值作为 `estimated`，
    /// 否则自反馈会让校准收敛到 √(真实比值)。
    pub fn count_raw(&self, text: &str) -> usize {
        self.inner.count(text)
    }
}

/// 全局默认估算器（进程内单例）。
///
/// 用 `CalibratedTokenizer` 而非裸 `HeuristicTokenizer`：这样任何一次 provider 真实用量
/// 反馈（`report_provider_usage`）都会滚动修正全局估算，对中文 / 代码等系统性偏差收敛最快。
static DEFAULT: std::sync::OnceLock<CalibratedTokenizer> = std::sync::OnceLock::new();

pub fn default_tokenizer() -> &'static CalibratedTokenizer {
    DEFAULT.get_or_init(CalibratedTokenizer::new)
}

/// 用 provider 返回的真实输出用量校准全局估算器。
///
/// `estimated` 必须是**未经校准的原始启发式估算**（[`CalibratedTokenizer::count_raw`]），
/// `actual` 为 provider 给的 `usage.output_tokens`（可能为 `None`，此时忽略）。
pub fn report_provider_usage(estimated: usize, actual: Option<u32>) {
    if let Some(a) = actual {
        default_tokenizer().feedback(a as usize, estimated);
    }
}

// ==================== 头尾切分（预算的另一半：怎么花）====================
//
// 头尾切分共享机制 —— L0（工具结果守卫）与工具结果淡化（请求视图层）头尾保留
// 策略族的唯一机制实现。
//
// 「机制 vs 策略」边界（刻意为之，勿合并）：
// - **机制**（本节）：token 计量、行贪心累加、`token × 2 → 字符上限`换算、
//   单行超长的字符兜底截断（头保行首 / 尾保行尾）；
// - **策略**（各调用方）：触发条件（token 预算 vs 行数阈值）、头尾配比
//   （约 60/40 偏头部 vs 约 1/4:3/4 偏尾部）、占位符文案（字符兜底的
//   `…` 截断标记属机制契约，由 [`split_head_tail`] 统一添加）。
//
// 策略分歧是有依据的设计（L0 面对工具 dump，结论/错误常在头部自报家门；
// 会话消息尾部离当前对话点更近），不得以"去重"为名强行统一；
// 机制收敛于此，保证策略族无法各自漂移（历史教训：曾有一处内联
// 重写同型逻辑，注释声称"对齐 L0 策略"实则已静默分叉）。

/// 字符换算系数：字符预算 ≈ token 预算 × 2（CJK 友好的粗略口径，全库统一）。
const CHARS_PER_TOKEN: usize = 2;

/// [`split_head_tail`] 的切分结果。
pub(crate) struct HeadTailSplit {
    /// 头部保留文本（每行自带换行；首行字符兜底时以 `…\n` 结尾）。
    pub head: String,
    /// 尾部保留文本（行间以 `\n` 连接，无尾换行；尾行字符兜底时以 `…` 开头）。
    pub tail: String,
}

/// 头部字符兜底：保留**行首** `token_budget × 2` 个字符。
///
/// 供单行超预算时使用——按行截断会"首行整行放行"导致压缩失效
/// （真实会话实证：12k token 的单行 glob 结果只省略了 1/3）。
/// 按字符而非字节截取，多字节 CJK 安全。
pub(crate) fn truncate_head_chars(text: &str, token_budget: usize) -> String {
    let char_cap = token_budget.saturating_mul(CHARS_PER_TOKEN);
    text.chars().take(char_cap).collect()
}

/// 尾部字符兜底：保留**行尾** `token_budget × 2` 个字符（结论/错误多在尾部）。
pub(crate) fn truncate_tail_chars(text: &str, token_budget: usize) -> String {
    let char_cap = token_budget.saturating_mul(CHARS_PER_TOKEN);
    let skip = text.chars().count().saturating_sub(char_cap);
    text.chars().skip(skip).collect()
}

/// 超限时截行首并追加省略号，未超限原样返回（不加省略号）。
///
/// 与 [`truncate_head_chars`] 的区别：自带"是否超限"判定，供以整段保留文本
/// 为对象的兜底使用（L3 骨架摘要的单行/单段截断）。
pub(crate) fn truncate_head_chars_if_over(text: &str, token_budget: usize) -> String {
    let char_cap = token_budget.saturating_mul(CHARS_PER_TOKEN);
    if text.chars().count() <= char_cap {
        text.to_string()
    } else {
        format!("{}…", truncate_head_chars(text, token_budget))
    }
}

/// 按 token 预算截断文本：未超限原样返回，超限截行首并追加省略号。
///
/// [`truncate_head_chars_if_over`] 的语义化别名——L3 骨架摘要等"单行/单段
/// 摘要"场景以此命名更贴近意图。历史上 context_window 内联了同型实现
/// （`token × 2 → 字符上限`），现收敛于此，杜绝口径漂移。
pub(crate) fn truncate_tokens(text: &str, token_budget: usize) -> String {
    truncate_head_chars_if_over(text, token_budget)
}

/// token 预算驱动的头尾切分（机制本体，原 L0 `tool_result_guard::split_head_tail`）。
///
/// 优先按**行**贪心累加（结构化输出友好，不会切断 JSON/表格行）；单行超预算
/// 退化为按字符截断（头保行首 / 尾保行尾）。字符兜底的 `…` 截断标记由本函数
/// 统一添加（机制契约，与原 L0 行为逐字节一致）；占位符文案属调用方策略。
///
/// 计量器由调用方注入：生产方传 `default_tokenizer()`（与预算计算共用同一
/// 校准口径）；本函数因此是**纯函数**——不读进程级校准状态，测试可用
/// 确定性的 [`HeuristicTokenizer`] 复现任意切分结果（否则测试会因其他用例
/// 的 usage 反馈漂移而 flaky，实测踩坑）。
pub(crate) fn split_head_tail(
    text: &str,
    head_budget: usize,
    tail_budget: usize,
    tok: &dyn Tokenizer,
) -> HeadTailSplit {
    let lines: Vec<&str> = text.lines().collect();

    // head：从前往后贪心累加，直到超出 head_budget
    let mut head = String::new();
    let mut used = 0usize;
    let mut head_line_count = 0usize;
    for &line in &lines {
        let c = tok.count(line) + tok.count("\n");
        if used + c > head_budget {
            if head.is_empty() || head_line_count == 0 {
                // 首行（或首个待收行）超预算：按字符兜底截行首，而非整行放行
                //（与原 L0 一致：截断标记 `…` + 换行，保持"每行自带换行"不变量）
                head.push_str(&truncate_head_chars(line, head_budget));
                head.push('…');
                head.push('\n');
            }
            break;
        }
        used += c;
        head.push_str(line);
        head.push('\n');
        head_line_count += 1;
    }

    // tail：从尾部（跳过已被 head 取走的部分）往回贪心累加
    let mut tail_lines: Vec<String> = Vec::new();
    let mut used_t = 0usize;
    for &line in lines[head_line_count.min(lines.len())..].iter().rev() {
        let c = tok.count(line) + tok.count("\n");
        if used_t + c > tail_budget {
            if tail_lines.is_empty() {
                // 尾行超预算：按字符兜底保行尾（`…` 前缀标记截断，与原 L0 一致）
                tail_lines.push(format!("…{}", truncate_tail_chars(line, tail_budget)));
            }
            break;
        }
        used_t += c;
        tail_lines.push(line.to_string());
    }
    tail_lines.reverse();

    HeadTailSplit {
        head,
        tail: tail_lines.join("\n"),
    }
}

#[cfg(test)]
#[path = "tokenizer.test.rs"]
mod tests;
