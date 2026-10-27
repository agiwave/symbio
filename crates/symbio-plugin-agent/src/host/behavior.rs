//! `agent_run` 的**效果**：一次委托就是一次跨主体立约（DE-025 D-207）。
//!
//! ## 为什么它住这里，而不是 `loop` 的工具执行层
//!
//! 「哪一次工具调用算立约、承诺的是哪句话」是**本能力的实现细节**：它读的是
//! `agent_run` 自己的参数形状（`prompt`），不是架构词汇。把这段判断留在宿主，
//! 每加一个带状态的能力就要在 `loop` 里再写一段按工具名的分支——那正是
//! [DE-025](../../../../docs/003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面") 要消灭的残余。
//!
//! 挪出来之后，宿主只做两件事：把「这次调用发生了什么」（`EffectInput`）交给写口，
//! 由写口问本模块要草稿、再经唯一写入口落格。宿主不认识 `agent_run`。
//!
//! ## 成格形状仍归 core
//!
//! 立约 / 守约 / 违约 / 宣告四格由 [`symbio_core::commitment_events`] 造——那是
//! **判断种类**（承诺是核心语义，读侧 `commitment_records` 与它配对），不是本能力的
//! 实现细节。本模块只负责「这一次调用对应哪一次立约、履约了没有」。
//!
//! ## 承诺号带轮锚
//!
//! `v2c-{轮锚}-{调用 id}`：tool_call id 只在一次模型响应内唯一，而 WAL 的幂等键是
//! 事件 id——跨轮复用同一个号会让第二次立约撞 `Duplicate`、把整轮收束拖失败。
//!
//! ## 没跑到的调用同样是没履约
//!
//! 协议失败 / 超限丢弃 / 被闸门拒止 / 批被中止的 `agent_run` 都走 `success = false`：
//! 「约了没履约」比「查无此约」诚实（S08 §5——违约可被观测是 T5 的全部前提）。
//! 待用户审批的不在此列：承诺还没到可判时刻，抢跑记成履约或违约都是假事实。

use symbio_core::commitment_events;
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{EffectCall, FactDraft, ToolBehavior};

/// 委托型工具的立约效果。
pub struct DelegationPromise;

impl ToolBehavior for DelegationPromise {
    fn owner(&self) -> &'static str {
        // 与 `PLUGIN.yml` 的 `plugin_id` 同一串：owner 序即装配序。
        "agent"
    }

    fn behavior(&self) -> &'static str {
        behaviors::DELEGATION_PROMISE
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        // 承诺内容 = 委托出去的那句话。参数里没有它 ⇒ 这次调用没有立约
        // （例如参数被截断成空对象：那种情况 `success` 也是 false，但承诺文本
        // 无从说起，记一条空承诺只会让读侧把违约算在「没说过的话」上）。
        let Some(promise) = call.args.get("prompt").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let cid = format!("v2c-{}-{}", call.anchor(), call.call_id);
        commitment_events(
            &cid,
            call.principal,
            call.source,
            promise,
            call.success,
            call.why,
            call.user_seq,
        )
        .into_iter()
        .map(FactDraft::from_event)
        .collect()
    }
}

symbio_plugin_api::submit_behavior!(DelegationPromise);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
