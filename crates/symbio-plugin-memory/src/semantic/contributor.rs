//! 收束贡献：语义记忆 —— 本轮检索读数落成一格 `memory.recalled`
//! （DE-036 §3 / DE-025 D-250 / DE-026 D-264 第 4 条）。
//!
//! ## 三个读数，全是**誊抄**
//!
//! | 键 | 取值 | 为什么它必须是事实 |
//! |---|---|---|
//! | `found` | 摘除之后的条目数 | 「这一轮问过记忆、拿回几条」是可审计的量，不是渲染细节 |
//! | `top` | 首条正文 | 首条是模型最先看见的那一句；空集时为空串 |
//! | `degraded` | 读数自带的那一格 | **降级的生产读数口**——只活在端口或提示词里，就没有一条事件说得出「这一轮记忆只显示了一部分」 |
//!
//! Translator 不创造信息（[DE-001 §9.2]）：三格全部来自 [`RecallFacts`]，本模块不重算
//! 折叠、不改排序、不判相关。重算就是一轮两个时刻两套读数。
//!
//! ## 幂等
//!
//! 键 `recalled-{user_seq}`：同一轮开口只落一条。重入（同一轮被重复收束）时按
//! `at.history` 里的同锚事实跳过——**不是**扫 store 句柄（贡献面没有 store，D-252）。
//!
//! ## 归属人
//!
//! `actor` = 本轮主体（宿主缺省填），**不是**认知归属人：这条事实说的是「这次会话向
//! 记忆问过什么」，问的人是跑这一轮的那个主体。与 `memory.encoded` 的归属人相反——
//! 编码说的是「谁记下了这件事」，那条归人。

use serde_json::json;
use symbio_plugin_api::{
    Contribution, Contributor, Entity, FactDraft, RecallFacts, Settling, Verb,
};

use super::decl::EVENT_MEMORY_RECALLED;
use crate::decl::OWNER;

pub(crate) struct SemanticRecall;

impl Contributor for SemanticRecall {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn contribute(&self, at: &Settling<'_>) -> Vec<Contribution> {
        // 本轮没走检索（答话轮 / 没有活记忆）⇒ 不落笔。空是合法结局，不是失败。
        let Some(recall) = at.recall else {
            return Vec::new();
        };
        // 同一轮开口只落一条：重入时史里已经有这一锚。
        if at
            .history
            .iter()
            .any(|e| e.kind == EVENT_MEMORY_RECALLED && e.produced_by == Some(at.user_seq))
        {
            return Vec::new();
        }
        vec![Contribution::new(draft(recall, at.user_seq))]
    }
}

/// 读数 → 一条待落格的草稿（信封 `turn` / `ts` / `actor` 由宿主盖章）。
fn draft(recall: &RecallFacts<'_>, user_seq: u64) -> FactDraft {
    FactDraft {
        key: format!("recalled-{user_seq}"),
        kind: EVENT_MEMORY_RECALLED.to_string(),
        entity: Entity::Memory,
        verb: Verb::Asserted,
        payload: json!({
            "found": recall.lines.len(),
            // 首条正文；空集时为空串（与旧写方逐字同形——不发明 `null`）。
            "top": recall.lines.first().map(|l| l.content).unwrap_or(""),
            "degraded": recall.degraded,
        }),
        // I2：断言类必带溯源。锚 = 触发检索的那格用户发言。
        produced_by: Some(user_seq),
    }
}

// 链接期提交：宿主按 `(prio, owner)` 询问。
symbio_plugin_api::submit_contributor!(SemanticRecall);

#[cfg(test)]
#[path = "contributor.test.rs"]
mod tests;
