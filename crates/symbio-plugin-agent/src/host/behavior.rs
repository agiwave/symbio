//! `agent_run` 的**效果与前置判断**：一次委托就是一次跨主体立约（DE-025 D-207），
//! 而被收回 `assign.work` 的主体派不活（D-219）。
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
//! 「约了没履约」比「查无此约」诚实（DE-038 §5——违约可被观测是 T5 的全部前提）。
//! 待用户审批的不在此列：承诺还没到可判时刻，抢跑记成履约或违约都是假事实。

use symbio_core::commitment_events;
use symbio_plugin_api::behaviors;
use symbio_plugin_api::effects::{
    BeforeDecision, BeforeRefusal, BeforeRequest, EffectCall, FactDraft, ToolBehavior,
};

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
            COMMITMENT_VOCAB,
            &cid,
            call.principal,
            call.source,
            promise,
            if call.success { Ok(()) } else { Err(call.why) },
            call.user_seq,
        )
        .into_iter()
        .map(FactDraft::from_event)
        .collect()
    }
}

/// 承诺面词汇表（DE-024 D-203 / D-206）：本插件写 `commitment.*` 四格里的三格，
/// 名字的 owner 是 `crates/symbio-plugin-loop`（它声明格位与角色），本插件经清单生成物取用
/// ——E-009 禁兄弟插件依赖，故不引用 loop 的 `control_decl`。
///
/// 表小到只有四个字段，正是它存在的理由：本插件只做「立约 / 履约 / 违约」这一件事，
/// 不该收到整面发言词汇。
const COMMITMENT_VOCAB: symbio_core::CommitmentVocab = symbio_core::CommitmentVocab {
    opened: symbio_plugin_api::events::EVENT_COMMITMENT_OPENED,
    released: symbio_plugin_api::events::EVENT_COMMITMENT_RELEASED,
    broken: symbio_plugin_api::events::EVENT_COMMITMENT_BROKEN,
    asserted: symbio_plugin_api::events::EVENT_COMMITMENT_ASSERTED,
};

symbio_plugin_api::submit_behavior!(DelegationPromise);

/// 淘汰回路的前置判断：主体被收回 `assign.work` ⇒ 本次委派不执行（DE-043 §3）。
///
/// ## 为什么判断住这里
///
/// 「`agent_run` 受淘汰管辖」是**本能力的声明**（`CapabilityMeta.behaviors` 上的
/// `assign.work`），判断读的也是本能力的语义（派活）。住在宿主时，这段逻辑按工具名
/// 分支；搬进来之后，宿主只交批首读好的结论（`BeforeRequest`），连 `agent_run` 这个
/// 名字都不认识（DE-025 D-219）。
///
/// ## 出口
///
/// 可审计拒绝，**记违约承诺**：这次调用确实约了派活却没派成（DE-038 §5）。判断本身
/// 不产事件——`verdict.asserted` 早已在事实源里，本层只是读侧（DE-043 §3）。
pub struct DelegationGate;

impl ToolBehavior for DelegationGate {
    fn owner(&self) -> &'static str {
        "agent"
    }

    fn behavior(&self) -> &'static str {
        behaviors::ASSIGN_WORK
    }

    fn before(&self, req: &BeforeRequest<'_>) -> BeforeDecision {
        if !req.readings.assign_work_revoked {
            return BeforeDecision::Allow;
        }
        BeforeDecision::Refuse(BeforeRefusal::new(
            "淘汰",
            format!(
                "主体 {} 的 assign.work 已被淘汰回路收回（verdict.asserted，声誉低于阈值），本次委派未执行",
                req.principal
            ),
            symbio_core::failure_kind::PERMISSION_DENIED,
        ))
    }
}

symbio_plugin_api::submit_behavior!(DelegationGate);

#[cfg(test)]
#[path = "behavior.test.rs"]
mod tests;
