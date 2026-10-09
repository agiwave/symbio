//! 轮边界的两个产物（缺口 4）。
//!
//! ## 为什么是「草稿」而不是「已落的事实」
//!
//! 汇报要落事实网格，而**开着 `EventWalStore` 的是 `v2_exec`**——判定与措辞在
//! `chat_loop`（它有编排器、有 compose），落格在 `v2_exec`。两边各管一半，所以
//! 回调交回来的只能是**草稿**：文本 + 它据以生成的运行现状。
//!
//! 交出「已落的事实」会让落格方无从判断真假；交出「文本」又会让判定方以为
//! 「说出来就等于记下了」——而这两件事在 v1 里从来不是同一件事（v1 的汇报节点
//! 落进 `context.messages`，由 `persist_messages` 落库，**不进事实网格**）。
//!
//! ## `into_message`：为什么汇报也要一条 `ChatMessage`
//!
//! 用户得看得见自己被汇报了，而前端读的是 `messages.json`。事实网格那一格是给
//! 模型与 `session/stats` 用的；**呈现那一格还得照旧给**。两条路各写一份不是
//! 「两份真源」——它们的内容由同一个 `ReportDraft` 生成，且都只写不改
//! （append-only）。
//!
//! `exclude_from_context: true` 与 `chat_loop/compose.rs` 的 `dialog_node` 逐字同义：
//! 汇报是界面文本，不进模型的请求包（否则线上出现连续两条 assistant）。

use std::sync::Arc;

use crate::symbio_core::chat_message::ChatMessage;

/// 轮边界回调：core 每跑完一轮工具问一次，返回「这一轮要不要说点什么」。
///
/// 形态与 [`crate::plugins::session::turn_runner::RoundInjector`] 同源（异步闭包 + `Fn + Send + Sync +
/// 'static`）而返回值不同：那条是「往请求包里加什么」，这条是「说一句什么」。
/// 刻意**不合成一条**：合起来就得靠「返回值有时有意义有时没有」来区分，那在
/// 调用点读不出来。
pub(crate) type RoundHookFn = Arc<
    dyn Fn(
            Option<u64>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<ReportDraft>> + Send>>
        + Send
        + Sync,
>;

/// 一句汇报的草稿：文本 + 它据以生成的运行现状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportDraft {
    /// 汇报正文（compose 的模板产线，零 LLM 往返）。
    pub text: String,
    /// 说这句话时已完成了几轮工具调用。
    pub tool_rounds: usize,
    /// 说这句话时距上次「用户看得见的一句话」多久。
    pub quiet_ms: i64,
}

impl ReportDraft {
    /// 落库镜像上的那一格（前端读它）：`dialog_node(text, "progress", true)`。
    ///
    /// 为什么**不复抄** `dialog_node` 的构造：上一版抄的漏了 `msg_type`
    /// ——节点落库了却不成形状，e2e 才照得出来。构造只有一份，漂移才
    /// 不可能；`SURFACE_REPLY` / `REASON_PROGRESS` 两个常量留在判定点
    /// 那侧，由函数签名（`reason: &str`）承担跨模块契约。
    pub(crate) fn to_message(&self) -> ChatMessage {
        super::chat_loop::dialog_node(&self.text, "progress", true)
    }
}

/// 把两个可选口打成一个可选项。
///
/// 为什么要有这个函数：`inject` 的构造点需要一个「要么有、要么没有」的闭包，
/// 而两个口各自都可能为 `None`（不挂抽干 / 不挂汇报）。写成两个 `map` 嵌套会得到
/// 四种组合，而其中一种（两个都没有）本来就该退化成 `None`——那种情况下挂一个
/// 永远返回空的闭包，白白让 core 每轮问一次。
pub(crate) fn round_hook(
    supplements: Option<crate::plugins::session::v2_exec::SupplementFn>,
    on_round: Option<RoundHookFn>,
) -> Option<(
    crate::plugins::session::v2_exec::SupplementFn,
    Option<RoundHookFn>,
)> {
    if supplements.is_none() && on_round.is_none() {
        return None;
    }
    let drain: crate::plugins::session::v2_exec::SupplementFn =
        supplements.unwrap_or_else(|| Arc::new(|| None));
    Some((drain, on_round))
}
