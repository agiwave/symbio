//! 上下文贡献：任务调度候选集段（DE-025 D-208 / D-215 / D-220）。
//!
//! provider 是**纯函数**：入参是 `ContextRequest` 里的值（本轮事实切片 + 已判过的召回
//! 视图），出参是一段文本；它不写事实、不改本轮状态，也不参与「欠什么 / 何时停」的
//! 判断。容量从**本轮的配置事实**读（[`crate::task::readyset_cap`]，DE-026 D-252）：
//! 它是宿主已经落格的决定，不是端口的一格——「哪些任务就绪」仍由 [`crate::task`] 的
//! 折叠回答，provider 只做排版。
//!
//! 段身份 [`symbio_plugin_api::context::slots::READYSET`] 是协议登记过的名字；
//! 本插件只负责在「有就绪任务」时把它交出来，没有就整段省略（不发空占位）。

use symbio_plugin_api::context::{self, ContextProvider, ContextRequest, ContextSection};

use crate::task;

/// 段优先级（D-243）：预算放不下时先丢谁，数大者先留。
///
/// 候选集可以从 `readyset` 视图重算，丢一段不至于让模型误判（召回段是背景事实，
/// 优先级更高）。
const PRIO: u8 = 70;

pub(crate) struct ReadySetProvider;

impl ContextProvider for ReadySetProvider {
    fn owner(&self) -> &'static str {
        "task"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match task::prompt_section(req.facts, task::readyset_cap(req.facts)) {
            Some(text) => vec![ContextSection::new(context::slots::READYSET, text, PRIO)],
            None => Vec::new(),
        }
    }
}

symbio_plugin_api::submit_context_provider!(ReadySetProvider);
