//! 本插件贡献的三段上下文：长期记忆召回、任务调度候选集与委派者真源
//! （DE-025 D-208 / D-215 / D-220）。
//!
//! ## 为什么它们在 `loop` 而不是在 `memory` / `session`
//!
//! 段的正文不是「记忆插件的数据」或「会话插件的数据」，而是**控制面判完之后要模型
//! 看见的那一段**：
//!
//! - 召回段读的是 `RecallView`，而那份视图已经被技能路由**摘掉过低置信的技能**
//!   （`skill_events::route`）——判定住控制面（D-208），provider 只渲染判完的值；
//! - 调度段读的是 `task_events` 的折叠，任务表的写方与调度者都在本插件
//!   （DE-024 D-206）；
//! - 委派段读的是 Q1 判完的结论 + 收集成形后的工具清单（`LiveInputs`）：判定在
//!   `prepare_request_sections`，provider 只排版。
//!
//! 三个 provider 都在 `owner() == "loop"` 下提交：同一 owner 内按提交序执行。前两段
//! 是轮首节奏、委派段是请求级节奏（D-220）——宿主先收轮首段、再收请求级段，于是
//! 「召回在前、调度在中、委派在后」与端口化之前逐字一致——**这次迁移的输出不变**，
//! 变的只是「谁决定有哪几段」。

use super::super::{chat_loop::delegate, memory_events, task_events};
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

/// 任务调度候选集段。
///
/// 读的是本轮事实切片 + 控制面给的容量（`task_cap`）：容量是配置投影，不是判据——
/// 「哪些任务就绪」由 `task_events` 的折叠回答，provider 只做排版。
pub(crate) struct ReadySetProvider;

/// 候选集可以从 `readyset` 视图重算，丢一段不至于让模型误判（见 [`RECALL_PRIO`]）。
const READYSET_PRIO: u8 = 70;

impl ContextProvider for ReadySetProvider {
    fn owner(&self) -> &'static str {
        "loop"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match task_events::prompt_section(req.facts, req.task_cap) {
            Some(text) => vec![ContextSection::new(
                context::slots::READYSET,
                text,
                READYSET_PRIO,
            )],
            None => Vec::new(),
        }
    }
}

/// 委派者真源段（Q1 + Q2 + Q3 合成一条）。
///
/// **请求级节奏**：Q2 的能力目录要的是收集完成后的工具清单，轮首拿不到（D-220）。
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

// 提交序 = 同节奏内的段序（同 owner 内按提交序执行）：召回在前、调度在中、委派在后。
symbio_plugin_api::submit_context_provider!(RecallProvider);
symbio_plugin_api::submit_context_provider!(ReadySetProvider);
symbio_plugin_api::submit_context_provider!(DelegateProvider);

#[cfg(test)]
#[path = "providers.test.rs"]
mod tests;
