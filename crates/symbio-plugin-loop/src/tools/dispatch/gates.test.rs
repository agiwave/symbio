//! 宿主闸表、拒止骨架与声明闸映射的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。这里钉的是**宿主闸本身**：
//! 闸名稳定且唯一、判据纯、每行只产出自己声明过的失败语义；声明闸（能力包的
//! `before`）到拒止骨架的映射也在这里——**两条出口由实现自报**，映射不猜。
//! 判据的业务场景（预算 / 淘汰 / 任务表）仍在 `tool_executor.test.rs` 走整批路径。

use super::*;
use symbio_core::{PluginInvokeRequestExt, PluginSimpleRequest};
use symbio_plugin_api::effects::BeforeRefusal;

fn neutral_breaker() -> BreakerInputs {
    BreakerInputs {
        authorized: true,
        spent_ms: 0,
        requested_ms: symbio_core::LatencyTier::Deep.budget_ms(),
        budget_ms: symbio_core::LatencyTier::Autonomic.budget_ms(),
    }
}

fn gate_ctx(breaker: &BreakerInputs) -> GateContext<'_> {
    GateContext {
        gate_started: std::time::Instant::now(),
        breaker,
    }
}

fn row(name: &str) -> &'static GateRow {
    GATES
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("闸门表里没有「{name}」"))
}

/// 闸名是外部可读的事实（日志 / 测试 / 审计），改名就是改事实。
///
/// 表里只剩外部执行：淘汰与任务表写侧是**能力包自己的判断**（`before`），
/// 不在这张宿主表里（DE-025 D-219）。
#[test]
fn gate_table_names_are_stable_and_unique() {
    let names: Vec<&str> = GATES.iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["外部执行"]);
    let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "闸名必须唯一");
}

/// 中性读数（授权、预算充足）⇒ 宿主闸放行。
///
/// 这条同时是判据纯度的底线：判据只在命中自己的条件时拒，不能因为「表里还有
/// 别的闸」而误伤。
#[tokio::test]
async fn neutral_context_passes_every_gate() {
    let breaker = neutral_breaker();
    let ctx = gate_ctx(&breaker);

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
    let ctx = gate_ctx(&breaker);

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
        budget_ms: symbio_core::LatencyTier::Deep.budget_ms() - 1,
        ..neutral_breaker()
    };
    let ctx = gate_ctx(&breaker);

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

/// 声明闸的默认出口是**可审计拒绝**：拒因进结果，违约承诺照记（淘汰即此类）。
#[test]
fn from_before_keeps_the_audited_default() {
    let judged = BeforeRefusal::new(
        "淘汰",
        "主体已收回".to_string(),
        failure_kind::PERMISSION_DENIED,
    );
    let refusal = Refusal::from_before(judged);
    assert_eq!(refusal.gate, "淘汰");
    assert_eq!(refusal.reason, "主体已收回");
    assert_eq!(refusal.prefix, "Refused");
    assert_eq!(refusal.kind, failure_kind::PERMISSION_DENIED);
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::Audited);
    assert!(refusal.trace.notes_delegation());
}

/// 实现显式去掉承诺记账 ⇒ 骨架不记（任务表：工具没跑，声明无从说起）。
#[test]
fn from_before_honours_the_implementations_trace() {
    let judged = BeforeRefusal::new(
        "任务表",
        "不持资格".to_string(),
        failure_kind::PERMISSION_DENIED,
    )
    .without_delegation_note();
    let refusal = Refusal::from_before(judged);
    assert_eq!(refusal.trace.kind(), RefusalTraceKind::AuditedNoDelegation);
    assert!(!refusal.trace.notes_delegation());
}

/// 主体派生（原 `EliminationInputs::of` 的那一条判据）：未绑智能体 ⇒ `session/main`；
/// 裸名归会话一族；带 namespace 的地址原样。
///
/// 声明闸与落笔读的是**同一个主体**——派生点因此留在批首读数里，而不是各闸自算。
#[test]
fn the_principal_comes_from_the_agent_id() {
    fn with_agent(agent_id: Option<&str>) -> std::sync::Arc<dyn PluginInvokeRequest> {
        let req = PluginSimpleRequest::new(None, None);
        if let Some(id) = agent_id {
            req.set(symbio_core::AGENT_ID, id.to_string());
        }
        std::sync::Arc::new(req)
    }

    assert_eq!(
        batch_principal(&with_agent(None)),
        symbio_core::PRINCIPAL_MAIN
    );
    assert_eq!(batch_principal(&with_agent(Some("ghost"))), "session/ghost");
    assert_eq!(
        batch_principal(&with_agent(Some("person/local"))),
        "person/local"
    );
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

// ==================== S09 外部执行闸门（纯函数，D-190） ====================
//
// 策略实现随 D-190 从 core 迁到本闸门文件；这里钉判定表与「熔断不静默」的
// 事件形状。整批路径（授权 → 闸门 → 拒止）仍在 `tool_executor.test.rs`。

/// DE-039 §6 验收 1：未授权 ⇒ `Refuse`（调用方因此零事件；事件面判据在整批路径里）。
/// 超时**先于**授权判定——判据顺序本身是规格。
#[test]
fn circuit_breaker_refuses_unauthorized() {
    assert_eq!(
        CircuitBreaker.gate(false, 0, 500, 1_000, 10, 80),
        GateDecision::Refuse
    );
    assert_eq!(
        CircuitBreaker.gate(false, 0, 500, 1_000, 200, 80),
        GateDecision::Break {
            reason: "gate-timeout"
        }
    );
}

/// DE-039 §6 验收 2 + 4：预算耗尽 / 超时熔断（都必须写事件）；预算放宽 ⇒ 放行。
#[test]
fn circuit_breaker_break_table_and_event() {
    let gate = CircuitBreaker;
    assert_eq!(
        gate.gate(true, 900, 500, 1_000, 10, 80),
        GateDecision::Break {
            reason: "budget-exhausted"
        }
    );
    assert_eq!(
        gate.gate(true, 900, 500, 10_000, 10, 80),
        GateDecision::Allow
    );
    assert_eq!(
        gate.gate(true, 0, 500, 10_000, 200, 80),
        GateDecision::Break {
            reason: "gate-timeout"
        }
    );

    // 熔断不许静默：`break_event` 必须落在 `control × opened`，载荷带 reason。
    let ev = gate.break_event("budget-exhausted", 7);
    assert_eq!(ev.kind, crate::control_decl::EVENT_TASK_CONTROLLED);
    assert_eq!(ev.entity, Entity::Control);
    assert_eq!(ev.verb, Verb::Opened);
    assert_eq!(ev.actor, PRINCIPAL_REFLEX);
    assert_eq!(ev.payload["reason"], "budget-exhausted");
}
