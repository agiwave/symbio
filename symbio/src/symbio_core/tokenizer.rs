//! Token 估算：上下文预算的计量基础
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

use std::sync::atomic::{AtomicU32, Ordering};

/// 单条消息的结构开销（role / 分隔符 / 消息边界），对齐主流 BPE 的经验值
pub const PER_MESSAGE_OVERHEAD: usize = 7;

/// 采样阈值：超过该字符数时先采样再外推，避免超大输入阻塞请求
const SAMPLE_CHARS: usize = 32_768;

/// token 计数抽象
pub trait Tokenizer: Send + Sync {
    /// 估算单段文本的 token 数
    fn count(&self, text: &str) -> usize;

    /// 估算消息列表（含每条消息的结构开销）
    fn count_messages(&self, msgs: &[crate::symbio_core::schemas::session::chat_message::ChatMessage]) -> usize {
        let mut total = 0;
        for m in msgs {
            total += PER_MESSAGE_OVERHEAD;
            if let Some(c) = &m.content {
                total += self.count(&c.to_text());
            }
        }
        total
    }

    /// 估算工具声明（function schema）的开销
    fn count_tools(&self, tools: &[crate::symbio_core::CapabilityMeta]) -> usize {
        let mut total = 0;
        for t in tools {
            total += self.count(&t.name) + self.count(&t.description) + 12;
        if !t.input_schema.is_null() {
            total += self.count(&t.input_schema.to_string());
        }
        }
        total
    }
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
    matches!(ch,
        '{' | '}' | '(' | ')' | '[' | ']' | '<' | '>' | ';' | '=' | '+' | '/' | '\\'
        | '|' | '&' | '*' | '!' | '?' | '#' | '$' | '@' | '~' | '^' | '`' | '_' | '%'
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
        let observed = ((actual as f64 / estimated as f64) * 10_000.0).clamp(2_000.0, 50_000.0)
            as u32;
        // alpha = 0.3 的指数滑动平均
        let cur = self.ratio.load(Ordering::Relaxed);
        let next = (cur as f64 * 0.7 + observed as f64 * 0.3) as u32;
        self.ratio.store(next.max(2_000), Ordering::Relaxed);
    }

    pub fn ratio(&self) -> f64 {
        self.ratio.load(Ordering::Relaxed) as f64 / 10_000.0
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

/// 当前校准系数（actual/estimate 的滑动均值），供诊断日志展示。
pub fn calibration_ratio() -> f64 {
    default_tokenizer().ratio()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_is_not_underestimated_by_byte_length() {
        // 回归守卫：若这里改回 text.len()/4，中文会被估成 0.75 token/字。
        let text = "你好世界";
        let est = default_tokenizer().count(text);
        // 4 个汉字 ≈ 4 token（外加 +1 兜底）
        assert!(
            est >= 4,
            "中文估算过低（{est}），疑似又退回按字节数估算"
        );
        // 也不应离谱高估
        assert!(est <= 12, "中文估算过高：{est}");
    }

    #[test]
    fn english_is_roughly_four_chars_per_token() {
        let text = "a".repeat(400);
        let est = default_tokenizer().count(&text);
        // 400 × 0.28 ≈ 112
        assert!((80..=200).contains(&est), "英文估算偏离预期：{est}");
    }

    #[test]
    fn count_messages_includes_overhead() {
        use crate::symbio_core::schemas::session::chat_message::{
            ChatMessage, MessageContent, MessageRole,
        };
        let msgs = vec![
            ChatMessage {
                role: Some(MessageRole::User),
                content: Some(MessageContent::Text("hi".into())),
                ..Default::default()
            },
            ChatMessage {
                role: Some(MessageRole::Assistant),
                content: Some(MessageContent::Text("hello".into())),
                ..Default::default()
            },
        ];
        let total = default_tokenizer().count_messages(&msgs);
        assert!(total >= 2 * PER_MESSAGE_OVERHEAD);
    }

    #[test]
    fn large_input_is_sampled_not_scanned() {
        let big = "a".repeat(2_000_000);
        let est = default_tokenizer().count(&big);
        // 2M × 0.28 ≈ 560k，采样外推应落在合理区间
        assert!((300_000..=900_000).contains(&est), "大输入外推异常：{est}");
    }

    #[test]
    fn calibration_converges_toward_actual() {
        let cal = CalibratedTokenizer::new();
        let text = "x".repeat(10_000);
        let est = cal.count(&text);
        // 假设真实值是估算的 2 倍（模拟中文场景的系统性低估）。
        // 反馈必须用「原始启发式估算」做分母（见 feedback 文档）：
        // 若用校准后的 count() 自反馈，比值会收敛到 √2 而非 2。
        let raw = cal.count_raw(&text);
        let actual = raw * 2;
        for _ in 0..40 {
            cal.feedback(actual, raw);
        }
        let after = cal.count(&text);
        assert!(
            after > est + est / 2,
            "校准未生效：before={est}, after={after}"
        );
    }
}
