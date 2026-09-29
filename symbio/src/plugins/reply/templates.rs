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
//! | `Report` | 模板（[`progress_text`]） | 事实已在 [`RunSnapshot`] 里，缺的只是把它说成人话——生成只会多一次**加在用户等待期间**的往返 |
//!
//! ## 兜底：未知码走通用模板
//!
//! 见 [`super::reasons`] 的说明——生产方加了码而这里忘了配措辞时，用户收到一句
//! 通用话而不是空白。**降级而不失效**。

use crate::symbio_core::schemas::dialog::{RunSnapshot, Verdict};

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
/// 返回 `None` 只有一种情况：[`Verdict::Report`]——它的正文随**运行现状**变，因此
/// 不在下面这张表里，而由 [`progress_text`] 填出来。表里的每一行都是"与上下文无关
/// 的固定措辞"，两者不是同一种数据。
pub(crate) fn template_for(verdict: &Verdict) -> Option<String> {
    let (reason, fallback) = match verdict {
        Verdict::Answered { reason } => (reason.as_str(), FALLBACK_ANSWERED),
        Verdict::Escalate { reason } => (reason.as_str(), FALLBACK_ESCALATE),
        Verdict::Report => return None,
    };
    Some(lookup(reason).unwrap_or(fallback).to_string())
}

/// `Report` 的措辞：把**运行现状**说成一句人话（**零 LLM 往返**）。
///
/// ## 为什么是填表而不是生成
///
/// 这句话的内容全部来自 [`RunSnapshot`] 的两个事实（跑了几轮、静默多久），措辞是
/// 固定的。让模型来写只会多出两样东西：一次**加在用户等待期间**的往返（汇报的全部
/// 意义是减少等待，不是延长它），以及"把 3 轮说成 4 轮"这种无从校验的漂移。
/// **事实已经在这里了，缺的只是把它说成人话。**
///
/// ## 为什么不说一句"我还在处理"
///
/// 那句信息量为零的话，用户从界面上的运行态就能看出来。本插件要说的是**"到哪一步了"**
/// ——那正是界面给不出的东西：工具节点讲的是"在跑什么"，而"一共走完了几轮、你等了
/// 多久"只有编排层知道。
///
/// ## 为什么有两句
///
/// `tool_rounds = 0` 在编排层不可达（汇报判定要求至少走完一轮，见 `session` 的
/// `chat_loop/progress.rs`），但本契约还有**第二个调用方**：网关把外部客户端的
/// `path` 原样转发给容器，`reply/compose` 可能被仓外程序直接调用。那句话在这里
/// 必须说得通，而不是渲染出"已完成 0 轮工具调用"。
pub(super) fn progress_text(snapshot: &RunSnapshot) -> String {
    let quiet = humanize_ms(snapshot.quiet_ms);
    if snapshot.tool_rounds == 0 {
        return format!("还在处理，已经 {quiet} 了，稍等一下。");
    }
    format!(
        "已经完成 {} 轮工具调用，用时约 {quiet}，还在继续。",
        snapshot.tool_rounds
    )
}

/// 毫秒 → 一句人话（秒 / 分钟两档）。
///
/// 只分两档、不写「1 小时 3 分 12 秒」：汇报要说的是"还在动、走到哪了"，精度在这里
/// 没有价值，位数越多越像机器自说自话。负数（时钟回拨）按 0 处理，并抬到 1 秒
/// ——"用时约 0 秒"读起来像故障。
fn humanize_ms(ms: i64) -> String {
    let secs = (ms.max(0) / 1_000).max(1);
    if secs < 60 {
        format!("{secs} 秒")
    } else {
        format!("{} 分钟", secs / 60)
    }
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
