//! 本插件贡献的上下文段：**委派者真源**（DE-025 D-208 / D-215 / D-220）。
//!
//! ## 召回段不在这里（L2b 之后）
//!
//! 长期记忆召回段的**排版**（抬头 / 降级措辞 / 字节上界 / 跨会话同文去重）与它的
//! provider 自 L2b 起住能力包（`crates/symbio-plugin-memory` 的 `recall::provider`，
//! DE-026 D-264）：段长什么样是检索读数的排版口径，不是控制面的判断。控制面只决定
//! 「这一段放不放」与「哪些条目过闸」（`run_recall` 的摘行），provider 收到的是已判过
//! 的值。
//!
//! 段的次序不变：owner 字典序 `loop` < `memory` < `task` ⇒ 发言级仍是
//! 「召回 → 调度」，请求级仍是「委派」——与召回段住本插件时逐字相同。
//!
//! ## 委派段为什么还在本插件
//!
//! 段读的是 Q1 判完的结论 + 收集成形后的工具清单（`LiveInputs`）：判定在
//! `prepare_request_sections`，provider 只排版。判定与排版同住控制面，因为它们说的
//! 是「本会话有哪些工具、worker 怎么起」——那是控制面的判断，不是任何一条能力的判据。

use super::super::chat_loop::delegate;
use symbio_plugin_api::context::{self, Cadence, ContextProvider, ContextRequest, ContextSection};

/// 委派者真源段（Q1 + Q2 + Q3 合成一条）。
///
/// **请求级节奏**：Q2 的能力目录要的是收集完成后的工具清单，发言级拿不到（D-220）。
/// 输入里的委派结论是控制面判完的值（`DelegateVerdict`）——provider 不读配置、
/// 不重判（D-208），只交给 [`delegate::delegate_section`] 排版。
pub(crate) struct DelegateProvider;

/// 委派段说明「本会话有哪些工具、worker 怎么起」：丢了会让模型对自己的能力面误判，
/// 只排在背景记忆之后（召回段的 90 住能力包，见模块文档）。
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

// 本插件只剩这一段：召回段由 `symbio-plugin-memory` 提交（owner 字典序把它排在前面），
// 调度段由 `symbio-plugin-task` 提交（排在后面）。
symbio_plugin_api::submit_context_provider!(DelegateProvider);

#[cfg(test)]
#[path = "providers.test.rs"]
mod tests;
