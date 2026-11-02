//! 批形状合同的**最小提供者夹具**：证明合同能被 `dyn` 侧实现并跑通一次空批，
//! 同时钉住三条读数口径（风险档 → 策略、端口并发 → 策略、未声明 → Medium）。

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use symbio_kernel::capability::{CapabilityMeta, CapabilityRiskLevel};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginSimpleRequest};

use crate::effects::BeforeReadings;
use crate::port::{ports, PortProvider};
use crate::tool_batch::{
    policy_for_port, policy_for_risk, BatchGateReadings, BehaviorDecls, ExecPolicy, GuardedResult,
    ToolBatchExecutor, ToolBatchInput, ToolBatchOutcome, ToolResultGuard, ToolRisks,
};

/// 最小提供者：申报 tool 端口，只实现批形状合同。
struct FixtureProvider;

impl PortProvider for FixtureProvider {
    fn port(&self) -> &'static str {
        ports::TOOL
    }

    fn provider_id(&self) -> &str {
        "fixture"
    }

    fn tool_batch(&self) -> Option<Arc<dyn ToolBatchExecutor>> {
        Some(Arc::new(FixtureBatch))
    }
}

/// 最小批实现：本夹具只验证「出参如实交回」，不替任何真实批行为作保。
struct FixtureBatch;

#[async_trait]
impl ToolBatchExecutor for FixtureBatch {
    async fn execute_batch(&self, input: ToolBatchInput) -> ToolBatchOutcome {
        assert!(input.calls.is_empty(), "空批不该带调用");
        ToolBatchOutcome::default()
    }
}

/// 恒等守卫：合同只要求「交文本、取回文本」，裁剪口径不属于本夹具。
struct IdentityGuard;

impl ToolResultGuard for IdentityGuard {
    fn guard(&self, text: &str) -> GuardedResult {
        GuardedResult {
            text: text.to_string(),
            truncated: false,
            original_tokens: 0,
            archive_path: None,
        }
    }
}

fn empty_input() -> ToolBatchInput {
    ToolBatchInput {
        calls: Vec::new(),
        readings: BatchGateReadings {
            mode: "auto".to_string(),
            gate_started: Instant::now(),
            breaker: Default::default(),
            decls: BehaviorDecls::default(),
            before: BeforeReadings::default(),
            principal: String::new(),
            session_facts: Vec::new(),
            risks: ToolRisks::default(),
            concurrency: 1,
            batch_meta: Vec::new(),
            seen_calls: Vec::new(),
        },
        guard: Arc::new(IdentityGuard),
        context_messages: Vec::new(),
        parent: None,
        ctx: Arc::new(PluginSimpleRequest::new(None, None)) as Arc<dyn PluginInvokeRequest>,
        sink: ExecEventSink::Null,
        abort: ExecAbortSignal::default(),
    }
}

#[tokio::test]
async fn 空批交回空出参() {
    let provider: Arc<dyn PortProvider> = Arc::new(FixtureProvider);
    let batch = provider.tool_batch().expect("tool 端口必须交出批合同");
    let outcome = batch.execute_batch(empty_input()).await;
    assert!(outcome.tool_messages.is_empty());
    assert!(outcome.parent_updates.is_empty());
    assert!(outcome.effects.is_empty());
    assert!(outcome.breaks.is_empty());
}

#[test]
fn 风险档映射只有低风险并行() {
    assert_eq!(policy_for_risk(CapabilityRiskLevel::Low), ExecPolicy::Parallel);
    assert_eq!(policy_for_risk(CapabilityRiskLevel::Medium), ExecPolicy::Serial);
    assert_eq!(policy_for_risk(CapabilityRiskLevel::High), ExecPolicy::Serial);
}

#[test]
fn 端口并发是硬闸() {
    // `concurrency <= 1` 压过一切：只读、自动模式也不行。
    assert_eq!(
        policy_for_port(1, "auto", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_port(0, "interactive", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_port(4, "interactive", ExecPolicy::Parallel),
        ExecPolicy::Serial
    );
    assert_eq!(
        policy_for_port(4, "auto", ExecPolicy::Parallel),
        ExecPolicy::Parallel
    );
}

#[test]
fn 未声明风险按中档兜底() {
    let metas = vec![CapabilityMeta {
        name: "read_only".to_string(),
        ..CapabilityMeta::default()
    }];
    let risks = ToolRisks::of_metas(&metas[..]);
    assert_eq!(risks.effective_risk("read_only"), CapabilityRiskLevel::Medium);
    assert_eq!(
        risks.policy_for_call("read_only"),
        ExecPolicy::Serial
    );
}
