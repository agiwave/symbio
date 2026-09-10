//! 头尾切分共享机制 —— L0（工具结果守卫）/ L1（消息存档）头尾保留策略族的唯一机制实现。
//!
//! 「机制 vs 策略」边界（刻意为之，勿合并）：
//! - **机制**（本模块）：token 计量、行贪心累加、`token × 2 → 字符上限`换算、
//!   单行超长的字符兜底截断（头保行首 / 尾保行尾）；
//! - **策略**（各调用方）：触发条件（L0 token 预算 vs L1 行数阈值）、头尾配比
//!   （L0 约 60/40 偏头部 vs L1 约 1/4:3/4 偏尾部）、占位符文案（字符兜底的
//!   `…` 截断标记属机制契约，由 [`split_head_tail`] 统一添加）。
//!
//! 策略分歧是有依据的设计（L0 面对工具 dump，结论/错误常在头部自报家门；
//! L1 面对会话消息，尾部离当前对话点更近），不得以"去重"为名强行统一；
//! 机制收敛于此，保证策略族无法各自漂移（历史教训：message_archive 曾内联
//! 重写一份同型逻辑，注释声称"对齐 L0 策略"实则已静默分叉）。

use super::tokenizer::Tokenizer;

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
/// 为对象的兜底使用（L1 消息存档的 token 上限兜底、L3 骨架摘要单行截断）。
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
mod tests {
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
}
