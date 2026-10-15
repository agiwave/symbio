//! 任务表写侧闸判据（S03 §3，T-107）——批首读一次的主体资格。
//!
//! 四条判据：**立任务者放行**（`define.work` 三持有者逐一）、**独立判定者放行**
//! （`assert.verification` 两持有者）、**纯产出 / 纯派活主体拒**（工人 / 对等体 / 市场）、
//! **工具名判据**（`local/todo_write` 认、`local/shell` 不认）。

use super::{is_task_table_tool, TaskTableInputs};
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

/// 未登记的 `agent:<id>`（子智能体）按 S08 §4 平凡值派生主智能体整套 ⇒ 照常放行。
///
/// 这是**子树委派侧**的口径（t42 的 `reviewer` 子会话就在这条路上）：子智能体要能
/// 写自己的收束格与任务表，否则什么都干不了。收窄它的正确位置是上游——登记窄行
/// （T-107 的三角色）或委派上限（T-105），不是这里。
#[test]
fn an_unregistered_subagent_keeps_the_derived_set() {
    let inputs = TaskTableInputs::of(&agent_ctx("reviewer"));
    assert_eq!(inputs.principal, "agent:reviewer");
    assert!(
        inputs.may_write,
        "未登记子智能体派生主智能体整套（含 define.work）⇒ 照常放行"
    );
}

/// 工具名判据：线上全名取末段（`local/todo_write` ⇒ `todo_write`），别的工具不拦。
///
/// 与 `note_tasks` 认名的口径逐字一致——出参见短名、闸门认全名就会漂移成
/// 「声明记了、闸门没拦」。
#[test]
fn the_tool_name_judgment_matches_the_declaration_reader() {
    assert!(is_task_table_tool("todo_write"));
    assert!(is_task_table_tool("local/todo_write"));
    assert!(!is_task_table_tool("local/shell"));
    assert!(!is_task_table_tool("todo_writer"));
    assert!(!is_task_table_tool(""));
}
