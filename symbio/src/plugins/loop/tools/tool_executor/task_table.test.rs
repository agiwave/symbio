//! 任务表写侧闸判据（S03 §3，T-107）——批首读一次的主体资格。
//!
//! 五条判据：**立任务者放行**（`define.work` 三持有者逐一）、**独立判定者放行**
//! （`assert.verification` 两持有者）、**纯产出 / 纯派活主体拒**（工人 / 对等体 / 市场）、
//! **未绑定登记的子智能体闸拒**、以及平凡值主体照常放行。
//! 「谁是任务表写入」不在这里判——它在工具自己的 `owns_task_table` 声明上
//! （T-158），见 `decls.test.rs` 的匹配口径用例。

use super::TaskTableInputs;
use crate::symbio_core::{authz, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest};

/// 带 `AGENT_ID` 的请求上下文：闸门按它派生主体（同 `EliminationInputs` 的取值口径）。
fn agent_ctx(agent_id: &str) -> std::sync::Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
    std::sync::Arc::new(req)
}

/// 无 `AGENT_ID` 的请求上下文：平凡值主体 = `agent:main`（生产里绝大多数会话的形态）。
fn bare_ctx() -> std::sync::Arc<dyn PluginInvokeRequest> {
    std::sync::Arc::new(PluginSimpleRequest::new(None, None))
}

/// `define.work` 的三个持有者都能立任务——现有流程一步都不能断。
///
/// 主智能体（自己的工作跟踪）与自主发起者（`AutonomousInitiator` 先例）是**已在生产**
/// 的两条路；规划者是 T-107 新登记的行。三者放行是「闸不改变现有行为」的判据。
#[test]
fn every_define_work_holder_may_declare_tasks() {
    for agent in ["main", "autonomous", "planner"] {
        let inputs = TaskTableInputs::of(&agent_ctx(agent));
        assert_eq!(
            inputs.principal,
            authz::principal_of(Some(agent)),
            "主体派生应与 request_principal 同源"
        );
        assert!(
            inputs.may_write,
            "持 define.work 的主体 `{agent}` 必须能写任务表"
        );
    }
}

/// 平凡值（无 `AGENT_ID`）⇒ 主智能体 ⇒ 照常放行。
///
/// 「未选智能体的会话」是生产的绝对多数，闸在这里必须恒放——fail-closed 的**结构**
/// 在别处（登记行收窄），不在平凡值上翻面。
#[test]
fn the_trivial_session_keeps_its_task_table() {
    let inputs = TaskTableInputs::of(&bare_ctx());
    assert_eq!(inputs.principal, authz::PRINCIPAL_MAIN);
    assert!(inputs.may_write, "平凡值会话（agent:main）必须能写任务表");
}

/// `assert.verification` 的两个持有者都能写任务表——独立判定是任务表上的另一半权威。
///
/// 仲裁者（T-073 淘汰回路）与验证者（T-107 新登记）同持验证能力而不同角色，
/// 放行判据相同：盖章的动作不论源自声誉裁决还是质量判定。
#[test]
fn every_assert_verification_holder_may_judge_tasks() {
    for agent in ["arbiter", "verifier"] {
        let inputs = TaskTableInputs::of(&agent_ctx(agent));
        assert!(
            inputs.may_write,
            "持 assert.verification 的主体 `{agent}` 必须能写任务表"
        );
    }
}

/// 纯产出 / 纯派活 / 纯判定主体**不写任务表**——C15 的精神在任务表这一层。
///
/// 工人与对等体只持 `produce.artifact`（交付物是产物格），市场只持 `assign.work`
/// （派活 ≠ 立活），守门者与抢占判定者只持 `judge.intent`（判来者意图 ≠ 立任务）。
/// 今天没有任何生产路径让这些主体调 `todo_write`——此闸防的是**未来**：角色行落地后
/// 一个绑 `worker` 的子会话调 `todo_write` 被诚实拒止，而不是静默以主智能体资格写下去。
#[test]
fn producer_only_and_dispatch_only_principals_are_refused() {
    for agent in ["worker", "peer-b", "market", "gatekeeper", "reflex"] {
        let inputs = TaskTableInputs::of(&agent_ctx(agent));
        assert!(
            !inputs.may_write,
            "主体 `{agent}` 不持 define.work / assert.verification，不得写任务表"
        );
    }
}

/// 未绑定登记的主体在合流面没有行 ⇒ 闸拒（[ADR-099] 未登记 = 拒 +
/// [ADR-100] 绑定登记前没有行）。
///
/// 子智能体要写任务表就得**先经会话装配点绑定登记**（根会话继承会话面缺省行、
/// 委派子会话继承父行，见 `authz::bind_inheriting`）——本用例走的是 `TaskTableInputs::of`
/// 直读合流面、**不经过装配点绑定**的那一支：没有登记动作就没有行，闸诚实拒止。
/// 与收束闸同一口径：「忘配/忘绑」的后果是一条会被看见的拒绝，不是一次静默超权。
#[test]
fn an_unbound_subagent_is_refused_the_task_table() {
    let inputs = TaskTableInputs::of(&agent_ctx("reviewer"));
    assert_eq!(inputs.principal, "session/reviewer");
    assert!(
        !inputs.may_write,
        "未绑定登记的主体在合流面没有行 ⇒ 不放行（ADR-099 / ADR-100）"
    );
}
