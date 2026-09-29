//! 模板表 —— 理由码 → 一段面向用户的文本（**数据 + 纯函数**，零 LLM 往返）。
//!
//! ## 为什么模板是"数据"而不是"机制"
//!
//! 表就是一张 `&[(&str, &str)]`，查表是一个 `iter().find()`。新增一类措辞 = 加一行，
//! 不动任何类型、不动 `symbio_core`（J1）。这与 [`super::super::triage`] 的规则表
//! 是同一个形状：**能用数据表达的，就不要写成分支**。
//!
//! ## 哪些理由码走模板、哪个走生成
//!
//! | 理由码 | 走哪条 | 为什么 |
//! |---|---|---|
//! | `greeting` / `thanks` / `ack` / `empty` | 模板 | 措辞与上下文无关，一次 LLM 往返买不到任何东西 |
//! | `clarify` / `refuse` | 模板 | 同上：这两句话的**内容**是固定的，模型只会把它写长 |
//! | `from_context` | **生成** | 「答案在对话里」——那段文本只能从对话线组织出来，模板给不了 |
//! | `needs_work` / `unclassified`（`Escalate`） | 模板 | 首响必须**立刻**出现（不等任何东西），模板是唯一能做到的形态 |
//!
//! ## 兜底：未知码走通用模板
//!
//! 见 [`super::reasons`] 的说明——生产方加了码而这里忘了配措辞时，用户收到一句
//! 通用话而不是空白。**降级而不失效**。

use crate::symbio_core::schemas::dialog::Verdict;

use super::reasons::{
    REASON_ACK, REASON_CLARIFY, REASON_EMPTY, REASON_GREETING, REASON_NEEDS_WORK, REASON_REFUSE,
    REASON_THANKS, REASON_UNCLASSIFIED,
};

/// 模板表：理由码 → 文本。
///
/// 键是**线上词汇表**（见 [`super::reasons`]），不是本地枚举——两个变体的码在这张表里
/// 不重叠，因此共用一张表不会歧义。
const TEMPLATES: &[(&str, &str)] = &[
    (REASON_GREETING, "你好，我在。有什么事直接说就行。"),
    (REASON_THANKS, "不客气。"),
    (REASON_ACK, "收到。"),
    (REASON_EMPTY, "你还没说要做什么——直接把要做的事告诉我就行。"),
    (REASON_CLARIFY, "我没太明白你的意思，能再说得具体一点吗？"),
    (REASON_REFUSE, "这件事我不合适做。换个方向我可以试试。"),
    // `Escalate` 的首响：一句话交代"我开始干了"，具体做什么由 worker 的正文说。
    (REASON_NEEDS_WORK, "好，我来处理。"),
    (REASON_UNCLASSIFIED, "我先看一下。"),
];

/// `Answered` 的兜底：模型/规则给出了一个**本插件不认识**的理由码。
///
/// 它必须是"能直接答"的口吻——绝不能是"我来处理"（那会让用户以为要干活，
/// 而本轮已经收尾了）。
pub(super) const FALLBACK_ANSWERED: &str = "好的。";

/// `Escalate` 的兜底：同样不认识的理由码。口吻必须与上一条相反——本轮**会**干活。
pub(super) const FALLBACK_ESCALATE: &str = "好，我来处理。";

/// 按判决取模板文本。
///
/// 返回 `None` 只有一种情况：[`Verdict::Report`]——它的措辞要从**运行现状**里组织
/// （在跑什么、跑了多久、完成了什么），而那个快照到 S4 才有生产者。本批**不编**
/// 一句话：编出来的那句必然与界面上的真实进展不符，比不说更糟。
pub(crate) fn template_for(verdict: &Verdict) -> Option<String> {
    let (reason, fallback) = match verdict {
        Verdict::Answered { reason } => (reason.as_str(), FALLBACK_ANSWERED),
        Verdict::Escalate { reason } => (reason.as_str(), FALLBACK_ESCALATE),
        Verdict::Report => return None,
    };
    Some(lookup(reason).unwrap_or(fallback).to_string())
}

/// 查表（纯函数，无兜底——兜底由调用方给，因为兜底**随变体而不同**）。
fn lookup(reason: &str) -> Option<&'static str> {
    TEMPLATES
        .iter()
        .find(|(code, _)| *code == reason)
        .map(|(_, text)| *text)
}

#[cfg(test)]
#[path = "templates.test.rs"]
mod tests;
