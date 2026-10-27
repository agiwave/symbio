//! 本插件贡献的两段上下文：长期记忆召回与任务调度候选集（DE-025 D-208 / D-215）。
//!
//! ## 为什么它们在 `loop` 而不是在 `memory` / `session`
//!
//! 段的正文不是「记忆插件的数据」或「会话插件的数据」，而是**控制面判完之后要模型
//! 看见的那一段**：
//!
//! - 召回段读的是 `RecallView`，而那份视图已经被技能路由**摘掉过低置信的技能**
//!   （`skill_events::route`）——判定住控制面（D-208），provider 只渲染判完的值；
//! - 调度段读的是 `task_events` 的折叠，任务表的写方与调度者都在本插件
//!   （DE-024 D-206）。
//!
//! 两个 provider 都在 `owner() == "loop"` 下提交：同一 owner 内按提交序执行，于是
//! 「召回在前、调度在后」与端口化之前逐字一致——**这次迁移的输出不变**，变的只是
//! 「谁决定有哪几段」。

use super::super::{memory_events, task_events};
use symbio_plugin_api::context::{self, ContextProvider, ContextRequest, ContextSection};

/// 长期记忆召回段。
///
/// 输入是**已判过**的召回视图（`ContextRequest::recall`），它到这一层只剩「渲染」：
/// 去重、按字节预算截断、加降级说明。判定（哪些条目可见、哪些技能被摘掉）在
/// `run_recall` 里已经做完——provider 无权改判，也不需要看见 `RecallView`。
pub(crate) struct RecallProvider;

impl ContextProvider for RecallProvider {
    fn owner(&self) -> &'static str {
        "loop"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match req.recall.and_then(memory_events::prompt_section) {
            Some(text) => vec![ContextSection {
                slot: context::slots::RECALL,
                text,
            }],
            None => Vec::new(),
        }
    }
}

/// 任务调度候选集段。
///
/// 读的是本轮事实切片 + 控制面给的容量（`task_cap`）：容量是配置投影，不是判据——
/// 「哪些任务就绪」由 `task_events` 的折叠回答，provider 只做排版。
pub(crate) struct ReadySetProvider;

impl ContextProvider for ReadySetProvider {
    fn owner(&self) -> &'static str {
        "loop"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match task_events::prompt_section(req.facts, req.task_cap) {
            Some(text) => vec![ContextSection {
                slot: context::slots::READYSET,
                text,
            }],
            None => Vec::new(),
        }
    }
}

// 提交序 = 段序（同 owner 内按提交序执行）：召回在前、调度在后。
symbio_plugin_api::submit_context_provider!(RecallProvider);
symbio_plugin_api::submit_context_provider!(ReadySetProvider);

#[cfg(test)]
#[path = "providers.test.rs"]
mod tests;
