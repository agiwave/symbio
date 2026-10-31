//! 召回段的上下文 provider（DE-025 D-208 / D-215 / D-220）。
//!
//! ## 为什么它在能力包里
//!
//! provider 渲染的是**检索读数**，而读数与它的排版口径都是检索能力的判据
//! （[`super::section`]）。段的位置与预算优先级是宿主的判断（`RECALL_PRIO`），
//! 但「这一段长什么样」不归控制面——否则每改一次措辞都要动控制面一次。
//!
//! 判定（哪些条目可见、哪些技能被摘掉）在宿主的 `run_recall` 里已经做完：provider
//! 收到的是 [`ContextRequest::recall`]（已判过的值），无权改判，也不需要看见
//! `Retrieval` 结构本身。
//!
//! ## owner 取插件 id
//!
//! 段的次序按 owner 字典序（DE-025 §3 第 4 条）：`loop` < `memory` < `task`。
//! 召回段仍在调度段之前、委派段之前——与它住 `loop` 时逐字相同的次序。

use symbio_plugin_api::context::{self, ContextProvider, ContextRequest, ContextSection};

/// 段优先级（D-243）：预算放不下时先丢谁，数大者先留。
///
/// 召回是**背景事实**：丢了它，模型不知道自己此前做过什么，本轮每一步都要重新试错。
const RECALL_PRIO: u8 = 90;

pub(crate) struct RecallProvider;

impl ContextProvider for RecallProvider {
    fn owner(&self) -> &'static str {
        "memory"
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        match req.recall.and_then(super::section::prompt_section) {
            Some(text) => vec![ContextSection::new(
                context::slots::RECALL,
                text,
                RECALL_PRIO,
            )],
            None => Vec::new(),
        }
    }
}

symbio_plugin_api::submit_context_provider!(RecallProvider);

#[cfg(test)]
#[path = "provider.test.rs"]
mod tests;
