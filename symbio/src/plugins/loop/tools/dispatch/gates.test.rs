//! 闸门表与拒止骨架的单元测试（T-196）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。这里钉的是**表本身**：
//! 闸名稳定且唯一、判据顺序（先命中者胜）、每行只产出自己声明过的失败语义。
//! 判据的业务场景（预算 / 淘汰 / 任务表）仍在 `tool_executor.test.rs` 走整批路径。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{
    Capability, CapabilityMeta, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequestExt,
    Registry,
};
use std::sync::Arc;

/// 只带元数据的假工具：注册面怎么读，测试就怎么喂（与 `factory_decls` 同形）。
struct DeclTool(CapabilityMeta);

#[async_trait::async_trait]
impl Capability for DeclTool {
    fn meta(&self) -> CapabilityMeta {
        self.0.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::Value::Null)
    }
}

/// 用注册面构造 `ToolDecls`：(名字, 提供 assign.work, 写任务表)。
async fn decls_with(entries: &[(&str, bool, bool)]) -> ToolDecls {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    let visitor = Arc::new(DefaultToolVisitor::new());
    for (name, assign_work, owns_task_table) in entries {
        visitor
            .put(CapabilityRow::tool(Arc::new(DeclTool(CapabilityMeta {
                name: (*name).to_string(),
                provides_assign_work: *assign_work,
                owns_task_table: *owns_task_table,
                ..Default::default()
            })) as Arc<dyn Capability>))
            .await;
    }
    ctx.set(
        crate::symbio_core::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    ToolDecls::of(&ctx).await
}

fn neutral_breaker() -> BreakerInputs {
    BreakerInputs {
        authorized: true,
        spent_ms: 0,
        requested_ms: crate::symbio_core::LatencyTier::Deep.budget_ms(),
        budget_ms: crate::symbio_core::LatencyTier::Autonomic.budget_ms(),
    }
}

fn neutral_elimination() -> EliminationInputs {
    EliminationInputs {
        principal: "agent:main".to_string(),
        delegation_revoked: false,
    }
}

fn neutral_task_table() -> TaskTableInputs {
    TaskTableInputs {
        principal: "agent:main".to_string(),
        may_write: true,
    }
}

fn gate_ctx<'a>(
    breaker: &'a BreakerInputs,
    elimination: &'a EliminationInputs,
    task_table: &'a TaskTableInputs,
    decls: &'a ToolDecls,
    name: &'a str,
) -> GateContext<'a> {
    GateContext {
        gate_started: std::time::Instant::now(),
        breaker,
        elimination,
        task_table,
        decls,
        name,
    }
}

fn row(name: &str) -> &'static GateRow {
    GATES
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("闸门表里没有「{name}」"))
}

/// 闸名是外部可读的事实（日志 / 测试 / 审计），改名就是改事实。
#[test]
fn gate_table_names_are_stable_and_unique() {
    let names: Vec<&str> = GATES.iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["外部执行", "淘汰", "任务表"]);
    let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "闸名必须唯一");
}

/// 中性读数（授权、预算充足、未淘汰、可写任务表、无声明）⇒ 每道闸都放行。
///
/// 这条同时是判据纯度的底线：判据只在命中自己的条件时拒，不能因为「表里还有
/// 别的闸」而误伤。
#[tokio::test]
async fn neutral_context_passes_every_gate() {
    let breaker = neutral_breaker();
    let elimination = neutral_elimination();
    let task_table = neutral_task_table();
    let decls = ToolDecls::default();
    let ctx = gate_ctx(&breaker, &elimination, &task_table, &decls, "shell");

    for gate in GATES {
        assert!(
            (gate.decide)(&ctx).is_none(),
            "中性读数不得被「{}」拒止",
            gate.name
        );
    }
    assert!(first_refusal(&ctx).is_none());
}

/// `Refuse`（未授权）是**零事件**：拒因只进结果 / 父节点，出参不记熔断。
#[tokio::test]
async fn external_gate_refuse_is_zero_event() {
    let breaker = BreakerInputs {
        authorized: false,
        ..neutral_breaker()
    };
    let elimination = neutral_elimination();
    let task_table = neutral_task_table();
    let decls = ToolDecls::default();
    let ctx = gate_ctx(&breaker, &elimination, &task_table, &decls, "vdfs_write");

    let refusal = first_refusal(&ctx).expect("未授权必须被拒");
    assert_eq!(refusal.gate, "外部执行");
    assert_eq!(refusal.kind, failure_kind::PERMISSION_DENIED);
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::ZeroEvent);
    assert!(
        row("外部执行").traces.contains(&refusal.trace.kind()),
        "拒绝语义必须在该行声明之内"
    );
    assert!(
        refusal.reason.contains("produce.artifact"),
        "拒因要说明缺的是哪项能力（实际: {}）",
        refusal.reason
    );
}

/// `Break`（预算耗尽）必须带理由：它经出参去重后入格，不允许静默继续。
#[tokio::test]
async fn external_gate_break_carries_the_reason() {
    let breaker = BreakerInputs {
        authorized: true,
        budget_ms: crate::symbio_core::LatencyTier::Deep.budget_ms() - 1,
        ..neutral_breaker()
    };
    let elimination = neutral_elimination();
    let task_table = neutral_task_table();
    let decls = ToolDecls::default();
    let ctx = gate_ctx(&breaker, &elimination, &task_table, &decls, "vdfs_write");

    let refusal = first_refusal(&ctx).expect("预算耗尽必须熔断");
    assert_eq!(refusal.gate, "外部执行");
    assert_eq!(refusal.kind, failure_kind::TOOL_UNAVAILABLE);
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::Recorded);
    assert!(
        matches!(&refusal.trace, RefusalTrace::Recorded("budget-exhausted")),
        "入格理由必须是去重键（实际: {:?}）",
        refusal.trace
    );
    assert!(row("外部执行").traces.contains(&refusal.trace.kind()));
}

/// 先命中者胜：同一个工具同时受淘汰与任务表管辖时，**淘汰先判**。
///
/// 顺序不是实现细节——它决定拒因写哪一条、走哪一类记账。表把它变成可断言的事实。
#[tokio::test]
async fn first_refusal_follows_the_table_order() {
    let decls = decls_with(&[("agent_run", true, true)]).await;
    let breaker = neutral_breaker();
    let elimination = EliminationInputs {
        principal: "agent:main".to_string(),
        delegation_revoked: true,
    };
    let task_table = TaskTableInputs {
        principal: "agent:main".to_string(),
        may_write: false,
    };
    let ctx = gate_ctx(&breaker, &elimination, &task_table, &decls, "agent_run");

    let refusal = first_refusal(&ctx).expect("两道闸都命中");
    assert_eq!(refusal.gate, "淘汰", "淘汰排在任务表之前");
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::Audited);
}

/// 任务表闸是**可审计拒绝但不记承诺**：工具没跑，声明无从说起。
#[tokio::test]
async fn task_table_gate_is_audited_without_delegation() {
    let decls = decls_with(&[("todo_write", false, true)]).await;
    let breaker = neutral_breaker();
    let elimination = neutral_elimination();
    let task_table = TaskTableInputs {
        principal: "agent:main".to_string(),
        may_write: false,
    };
    let ctx = gate_ctx(
        &breaker,
        &elimination,
        &task_table,
        &decls,
        "local/todo_write",
    );

    let refusal = first_refusal(&ctx).expect("任务表闸必须命中");
    assert_eq!(refusal.gate, "任务表");
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::AuditedNoDelegation);
    assert!(!refusal.trace.notes_delegation());
    assert!(row("任务表").traces.contains(&refusal.trace.kind()));
}

/// PreToolUse 钩子不进同步表，但共用同一套失败语义：`Blocked:` + `blocked` +
/// 违约理由保留钩子原措辞。
#[test]
fn blocked_refusal_keeps_hook_wording_and_semantics() {
    let refusal = Refusal::blocked("策略不允许".to_string());
    assert_eq!(refusal.gate, "PreToolUse");
    assert_eq!(refusal.prefix, "Blocked");
    assert_eq!(refusal.kind, "blocked");
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::Audited);
    assert_eq!(
        refusal.note_why.as_deref(),
        Some("被 PreToolUse 钩子拦下：策略不允许")
    );
}
