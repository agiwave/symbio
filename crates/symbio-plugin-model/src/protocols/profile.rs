//! 协议参数档案：供应商 / 协议差异的**唯一**落点。
//!
//! ## 为什么是一张表
//!
//! `max_tokens` 的缺省值与探测值放在这里，一行一个协议，每个数带一句「为什么是这个数」
//! （GLM 要求 `> 2`、Anthropic 要求 `>= 1`、Gemini 缺省 8192……）。它们是**协议约束**，
//! 属于本质复杂度——砍不掉，但可以只写一遍、只解释一遍。
//!
//! 判据是 [J1](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：
//! **能用已有参数的新取值表达的东西，就是数据不是机制**。新增一个整族兼容网关的差异
//! ⇒ 在下面加一行；协议解析代码一个字不动。
//!
//! ## 边界：这里只收「整族」差异
//!
//! 表按**协议**分行——那是今天唯一真实的维度。某一个**模型**的怪癖（某型号只认某个
//! 字段名、某个上下文上限）走 `ModelProviderConfig`，不进这张表：建一层没有条目的
//! vendor 覆盖机制，就是造一个「有形状、无数据」的假机制（[D-079](../../../../docs/003-design/005-协议规范.md#d-079-同名两物core-的-actorspecpattern-是主体分类de-041-4-的-actorpattern-是开关参数前者不该做分派点 "DE-005-协议规范")
//! 同一条纪律）。真出现「同一协议下两家必须给不同值」时再加维度，那时它有第一行数据。

use serde_json::Value;

/// 一条协议的输出上限档案。
///
/// 两个数各有各的用途，**不是一个全局常量**：生成要够大，探测要够小（省钱），
/// 而两者都必须过网关的下限。
pub(crate) struct MaxTokensProfile {
    /// 生成时调用方没给 `max_tokens` 的缺省值。
    pub default: u32,
    /// 连通性探测（`ping`）用的输出上限。
    ///
    /// 越小越省，但不能压到网关的下限以下：OpenAI 兼容生态里 GLM 要求 `max_tokens > 2`
    /// （设 1 会被拒），故探测值取 256；Anthropic 协议只要求 `>= 1`，取 16 足够。
    pub probe: u32,
}

/// OpenAI Chat Completions（含绝大多数兼容网关）。
pub(crate) const OPENAI_CHAT: MaxTokensProfile = MaxTokensProfile {
    default: 8192,
    probe: 256,
};

/// OpenAI Responses（`max_output_tokens` 最小 16）。
pub(crate) const OPENAI_RESPONSES: MaxTokensProfile = MaxTokensProfile {
    default: 8192,
    probe: 16,
};

/// Anthropic Messages（`max_tokens` 必填且 `>= 1`）。
pub(crate) const ANTHROPIC_MESSAGES: MaxTokensProfile = MaxTokensProfile {
    default: 8192,
    probe: 16,
};

/// Gemini API（`generationConfig.maxOutputTokens`）。
pub(crate) const GEMINI_API: MaxTokensProfile = MaxTokensProfile {
    default: 8192,
    probe: 16,
};

// ── 思考分片的字段名 ────────────────────────────────────────────────────

/// OpenAI 兼容生态里思考分片的字段名，按序尝试。
///
/// 这个生态**没有统一约定**：DeepSeek 风格用 `reasoning_content`，部分兼容端点用
/// `reasoning`——AMD Radeon AI 开发者平台的 MiMo-V2.6-Flash 实测（2026-10-13）即属后者，
/// 且首帧带 `reasoning_content: null`（字段存在但为空，不能当文本）。此前只认前者，
/// 该端点的思考内容被**静默丢弃**（Reason 节点从未出现）。
pub(crate) const OPENAI_CHAT_REASONING_KEYS: &[&str] = &["reasoning_content", "reasoning"];

/// Anthropic 风格：带类型的思考增量事件（`delta.type` 命中其一即取正文）。
pub(crate) const ANTHROPIC_THINKING_DELTA_TYPES: &[&str] =
    &["thinking_delta", "thought_delta", "reasoning_delta"];

/// Anthropic 风格：带类型事件里的正文字段名，按序尝试。
pub(crate) const ANTHROPIC_THINKING_KEYS: &[&str] = &["thinking", "thought", "reasoning"];

/// Anthropic 风格：正文增量里可能**夹带**的思考字段。
///
/// 部分兼容网关把思考塞进 `text_delta` 而不是发 `thinking_delta`。
pub(crate) const ANTHROPIC_TEXT_DELTA_REASONING_KEYS: &[&str] = &["reasoning_content"];

/// 按序取**第一个值是字符串**的键（`null` / 缺键视为未命中，继续下一个）。
///
/// 这是「fallback 链」的唯一实现：OpenAI 的扁平增量、Anthropic 的带类型增量、
/// 以及正文增量里夹带思考的那一处，都调它——三处各写一遍链式 `or_else` 迟早漂移成
/// 「一处认 `reasoning`、另一处不认」。
///
/// 不额外过滤空串：空串与 `null` 是两种不同的输入，前者今天会被当成一段空增量推出去
/// （行为如此），本函数不借「顺手清理」改变它。
pub(crate) fn first_str<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
}

#[cfg(test)]
#[path = "profile.test.rs"]
mod tests;
