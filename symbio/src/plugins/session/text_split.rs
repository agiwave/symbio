//! 头尾切分共享机制 —— L0（工具结果守卫）与工具结果淡化（请求视图层）头尾保留策略族的唯一机制实现。
//!
//! 「机制 vs 策略」边界（刻意为之，勿合并）：
//! - **机制**（本模块）：token 计量、行贪心累加、`token × 2 → 字符上限`换算、
//!   单行超长的字符兜底截断（头保行首 / 尾保行尾）；
//! - **策略**（各调用方）：触发条件（token 预算 vs 行数阈值）、头尾配比
//!   （约 60/40 偏头部 vs 约 1/4:3/4 偏尾部）、占位符文案（字符兜底的
//!   `…` 截断标记属机制契约，由 [`split_head_tail`] 统一添加）。
//!
//! 策略分歧是有依据的设计（L0 面对工具 dump，结论/错误常在头部自报家门；
//! 会话消息尾部离当前对话点更近），不得以"去重"为名强行统一；
//! 机制收敛于此，保证策略族无法各自漂移（历史教训：曾有一处内联
//! 重写同型逻辑，注释声称"对齐 L0 策略"实则已静默分叉）。

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
/// 确定性的 [`super::tokenizer::HeuristicTokenizer`] 复现任意切分结果（否则测试会因其他用例
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
#[path = "text_split.test.rs"]
mod tests;
