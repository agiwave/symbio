//! 本插件贡献的两段上下文：长期记忆召回与委派者真源（DE-025 D-208 / D-215 / D-220）。
//!
//! ## 为什么它们在 `loop`
//!
//! 段的正文不是「记忆插件的数据」或「会话插件的数据」，而是**控制面判完之后要模型
//! 看见的那一段**：
//!
//! - 召回段读的是 `RecallView`，而那份视图已经把**被技能判为低置信的条目摘掉**
//!   （`skill.retreat` 视图，D-262）——判定住能力包，provider 只渲染判完的值；
//! - 委派段读的是 Q1 判完的结论 + 收集成形后的工具清单（`LiveInputs`）：判定在
//!   `prepare_request_sections`，provider 只排版。
//!
//! 两个 provider 都在 `owner() == "loop"` 下提交：同一 owner 内按提交序执行。召回段
//! 是发言级节奏、委派段是请求级节奏（D-220）——宿主先收发言级段、再收请求级段，于是
//! 「召回在前、委派在后」与端口化之前逐字一致。
//!
//! 调度段不在这里：任务能力已迁出（`symbio-plugin-task`），段随它的能力包走。
//! 跨 owner 的次序按 owner 字典序，`loop` 先于 `task` ⇒ 「召回 → 调度 → 委派」的
//! 请求视图次序不变（D-220）。

use super::super::{chat_loop::delegate, memory_events};
use symbio_plugin_api::context::{self, Cadence, ContextProvider, ContextRequest, ContextSection};

/// 长期记忆召回段。
///
/// 输入是**已判过**的召回视图（`ContextRequest::recall`），它到这一层只剩「渲染」：
/// 去重、按字节预算截断、加降级说明。判定（哪些条目可见、哪些技能被摘掉）在
/// `run_recall` 里已经做完——provider 无权改判，也不需要看见 `RecallView`。
pub(crate) struct RecallProvider;

/// 段优先级（D-243）：预算放不下时先丢谁，数大者先留。
///
/// 召回是**背景事实**：丢了它，模型不知道自己此前做过什么，本轮每一步都要重新试错。
const RECALL_PRIO: u8 = 90;

impl ContextProvider for RecallProvider {
    fn owner(&self) -> &'static str {
        "loop"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match req.recall.and_then(memory_events::prompt_section) {
            Some(text) => vec![ContextSection::new(
                context::slots::RECALL,
                text,
                RECALL_PRIO,
            )],
            None => Vec::new(),
        }
    }
}

/// 委派者真源段（Q1 + Q2 + Q3 合成一条）。
///
/// **请求级节奏**：Q2 的能力目录要的是收集完成后的工具清单，发言级拿不到（D-220）。
/// 输入里的委派结论是控制面判完的值（`DelegateVerdict`）——provider 不读配置、
/// 不重判（D-208），只交给 [`delegate::delegate_section`] 排版。
pub(crate) struct DelegateProvider;

/// 委派段说明「本会话有哪些工具、worker 怎么起」：丢了会让模型对自己的能力面误判，
/// 只排在背景记忆之后（见 [`RECALL_PRIO`]）。
const DELEGATE_PRIO: u8 = 80;

impl ContextProvider for DelegateProvider {
    fn owner(&self) -> &'static str {
        "loop"
    }

    fn cadence(&self) -> Cadence {
        Cadence::Request
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        let Some(live) = req.live else {
            return Vec::new();
        };
        match delegate::delegate_section(live.delegate, live.tools, live.workers) {
            Some(text) => vec![ContextSection::new(
                context::slots::DELEGATE,
                text,
                DELEGATE_PRIO,
            )],
            None => Vec::new(),
        }
    }
}

// 提交序 = 同节奏内的段序（同 owner 内按提交序执行）：召回在前、委派在后。
// 调度段由 `symbio-plugin-task` 在发言级节奏下提交，owner 字典序把它排在 `loop` 之后。
symbio_plugin_api::submit_context_provider!(RecallProvider);
symbio_plugin_api::submit_context_provider!(DelegateProvider);

#[cfg(test)]
#[path = "providers.test.rs"]
mod tests;
