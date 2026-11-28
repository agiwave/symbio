//! `sdk/collectors/approval.rs` 的单元测试。
//!
//! 覆盖审批闸门的两处形状：
//! 1. **判定 [`approval_gate`]**——风险档与阈值的比较、模型 `approved` 无效、
//!    一次性 `ApprovalGrant` 的绑定/消费/重放/过期/版本校验、逐工具永久放行上限、
//!    沙箱单次裁决与 `mode=auto` 拒绝。
//! 2. **端口执行体 [`GatedExecutor`]**——判定是执行的第一段：拦下时停等载荷按
//!    `RESULT_KEY` 交回且**不落内层**；有效 typed grant 放行才透传给内层执行体。

use super::*;
use crate::kernel::capability::failure_kind;
use crate::kernel::exec::{ExecAbortSignal, ExecEventSink};
use crate::kernel::plugin::{PluginInvokeRequestExt, PluginSimpleRequest};
use crate::sdk::capability::port::{
    port_result, PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortInput,
    PortOutcome,
};
use crate::sdk::capability::{ApprovalGrant, PolicyBinding};
use crate::sdk::collectors::{
    approval_policy_init_slot, approval_policy_publish, ApprovalEvaluation, ApprovalPolicy,
    ApprovalScopes,
};
use crate::sdk::keys::{SANDBOX_ID, SESSION_ID, TOOL_CALL_ID};
use crate::sdk::sandbox::{sandbox_init_slot, sandbox_publish, SandboxDecision, SandboxFace};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// 最小可判定的执行体桩：固定风险、可选动态覆写、记录是否被真正执行。
struct StubTool {
    name: &'static str,
    risk: CapabilityRiskLevel,
    /// 动态风险覆写（如 shell 按命令分级）；`Some` 时优先于 `risk`。
    dyn_risk: Option<CapabilityRiskLevel>,
    /// 是否被真正执行过——闸门放行才该置位。
    executed: Arc<std::sync::atomic::AtomicBool>,
}

impl StubTool {
    fn new(name: &'static str, risk: CapabilityRiskLevel) -> Arc<Self> {
        Arc::new(Self {
            name,
            risk,
            dyn_risk: None,
            executed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    fn with_dyn_risk(mut self: Arc<Self>, r: CapabilityRiskLevel) -> Arc<Self> {
        // 借用不了 &mut，通过内部可变绕过——测试专用形状，不追求生产风格。
        Arc::get_mut(&mut self).unwrap().dyn_risk = Some(r);
        self
    }
    /// 判定用：从桩取风险回答（与注册收口同一个形状——闭包捕获自述，
    /// 动态覆写生效）。
    fn risk_of(tool: &Arc<Self>) -> RiskOf {
        let t = Arc::clone(tool);
        Arc::new(move |_args: &Value| t.dyn_risk.unwrap_or(t.risk))
    }
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.name.to_string(),
            description: format!("{} 的说明", self.name),
            input_schema: json!({"type":"object"}),
            risk: Some(self.risk),
            ..Default::default()
        }
    }
}

#[async_trait]
impl PortExecutor for StubTool {
    async fn execute(&self, exec: PortExecContext, _input: PortInput) -> PortOutcome {
        if exec.ctx.is_none() {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "没有调用方上下文".to_string(),
            );
        }
        self.executed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        port_result(Ok(json!({"ok": true})))
    }
}

/// 端口执行体用：同一形状裹出 `GatedExecutor`。
fn gated(tool: &Arc<StubTool>) -> Arc<GatedExecutor> {
    let inner: Arc<dyn PortExecutor> = tool.clone();
    Arc::new(GatedExecutor::new(
        tool.meta(),
        StubTool::risk_of(tool),
        inner,
    ))
}

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

struct FakeSandbox {
    decision: SandboxDecision,
    decide_calls: AtomicUsize,
}

impl FakeSandbox {
    fn new(decision: SandboxDecision) -> Arc<Self> {
        Arc::new(Self {
            decision,
            decide_calls: AtomicUsize::new(0),
        })
    }
}

impl SandboxFace for FakeSandbox {
    fn permits(&self, _facts: &[SandboxFact]) -> bool {
        self.decision.permitted
    }

    fn intercept(&self, _tool_name: &str, _args: &Value) -> Option<Value> {
        self.decision.intercept.clone()
    }

    fn decide(&self, _facts: &[SandboxFact], _tool_name: &str, _args: &Value) -> SandboxDecision {
        self.decide_calls.fetch_add(1, Ordering::SeqCst);
        self.decision.clone()
    }
}

fn ctx_with_sandbox(decision: SandboxDecision) -> (Arc<dyn PluginInvokeRequest>, Arc<FakeSandbox>) {
    let ctx = ctx();
    ctx.set(SANDBOX_ID, "default".to_string());
    let sandbox = FakeSandbox::new(decision);
    sandbox_init_slot(&ctx);
    assert!(sandbox_publish(&ctx, sandbox.clone()));
    (ctx, sandbox)
}

#[test]
fn explicit_sandbox_selection_without_a_handle_fails_closed() {
    let tool = StubTool::new("read_file", CapabilityRiskLevel::Low);
    let ctx = ctx();
    ctx.set(crate::sdk::keys::SANDBOX_ID, "strict".to_string());
    let payload = approval_gate(
        &ctx,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({}),
    )
    .expect("显式选档但句柄缺失必须拦截");
    assert_eq!(
        payload["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED)
    );
    assert!(payload["error"]
        .as_str()
        .is_some_and(|error| error.contains("沙箱句柄不可用")));
    assert!(!tool.executed.load(Ordering::SeqCst));
}

fn bind_call(ctx: &Arc<dyn PluginInvokeRequest>, call_id: &str, session_id: &str) {
    ctx.set(TOOL_CALL_ID, call_id.to_string());
    ctx.set(SESSION_ID, session_id.to_string());
}

/// 端口执行上下文：与派发侧同形（ctx 是唯一被用到的字段）。
fn exec_ctx(c: Arc<dyn PluginInvokeRequest>) -> PortExecContext {
    PortExecContext {
        abort: ExecAbortSignal::new(),
        idempotency_key: "test-1".to_string(),
        sink: ExecEventSink::Null,
        emit: PortFactEmit::silent(),
        ctx: Some(c),
        subject: String::new(),
        session_id: "s".to_string(),
        origin: None,
        scratch_dir: None,
        reader: None,
        source: String::new(),
    }
}

/// 执行体的出口约定：主结果值在 `facts[0]` 的 `RESULT_KEY` 里。
fn result_of(outcome: &PortOutcome) -> Value {
    outcome
        .facts
        .first()
        .and_then(|f| f.get(RESULT_KEY))
        .cloned()
        .unwrap_or(Value::Null)
}

// ==================== 判定：approval_gate ====================

/// 阈值缺席（默认 Medium）+ `risk = High` ⇒ 需要审批，产 `NEEDS_APPROVAL` 卡。
#[test]
fn high_risk_over_default_threshold_produces_confirm_card() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"path": "tmp/x"}),
    )
    .expect("High > Medium(默认) 应产卡");
    assert_eq!(
        payload.get("failure_kind").and_then(|v| v.as_str()),
        Some(failure_kind::NEEDS_APPROVAL),
    );
    assert!(
        payload.get("prompt").is_some(),
        "confirm 卡必须带 prompt 载荷"
    );
    assert_eq!(
        payload["prompt"]["tool_name"].as_str(),
        Some("vdfs_delete"),
        "卡上要能指认是哪个工具（前端呈现与恢复重放都靠它）"
    );
}

#[test]
fn approval_card_carries_tool_declared_summary_and_actual_target() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let summary = ApprovalSummary::targeted(
        "删除文件或目录",
        ApprovalEffect::FILE_DELETE,
        "path",
        "目标路径",
    );
    let payload = approval_gate_inner(
        &ctx(),
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"path": "tmp/old.txt"}),
        None,
        Some(&summary),
    )
    .expect("应产卡");
    assert_eq!(payload["prompt"]["approval"]["action"], "删除文件或目录");
    assert_eq!(payload["prompt"]["approval"]["effect"], "file_delete");
    assert_eq!(payload["prompt"]["approval"]["target_label"], "目标路径");
    assert_eq!(payload["prompt"]["approval"]["target"], "tmp/old.txt");
}

#[test]
fn approval_card_without_declared_summary_keeps_description_and_args() {
    let tool = StubTool::new("custom_tool", CapabilityRiskLevel::High);
    let payload = approval_gate(
        &ctx(),
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"custom": 1}),
    )
    .expect("应产卡");
    assert_eq!(payload["prompt"]["description"], "custom_tool 的说明");
    assert_eq!(payload["prompt"]["args"]["custom"], 1);
    assert!(payload["prompt"].get("approval").is_none());
}

/// 阈值缺席 + `risk = Low` ⇒ 放行。
#[test]
fn low_risk_passes_through_default_threshold() {
    let tool = StubTool::new("read_file", CapabilityRiskLevel::Low);
    let c = ctx();
    assert!(
        approval_gate(
            &c,
            &tool.meta(),
            StubTool::risk_of(&tool).as_ref(),
            &json!({})
        )
        .is_none(),
        "Low ≤ Medium(默认) 应放行"
    );
}

/// 模型参数 `approved: true` 对高风险调用完全无效；没有 typed grant 必须停等。
#[test]
fn raw_approved_argument_cannot_bypass_the_gate() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    bind_call(&c, "call-1", "session-1");
    c.set_raw(TRUSTED_APPROVED, Arc::new("true".to_string()));
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"path": "tmp/x", "approved": true}),
    )
    .expect("模型自称已批准不能放行");
    assert_eq!(
        payload["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL)
    );
    assert_eq!(
        c.get_raw(TRUSTED_APPROVED)
            .and_then(|value| value.downcast::<String>().ok())
            .as_deref()
            .map(ToString::to_string),
        Some("false".to_string()),
        "未消费 grant 时必须清掉继承来的旧标记"
    );
}

/// 有效 typed grant 放行，且只在 mandatory gate 消费一次；重放不能复用。
#[test]
fn valid_grant_passes_once_and_replay_is_consumed() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let (c, sandbox) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        profile_version: 7,
        ..Default::default()
    });
    bind_call(&c, "call-1", "session-1");
    let args = json!({"path": "tmp/x"});
    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &args,
        Some("session-1".into()),
        Some(PolicyBinding::new("default", 7)),
        Duration::from_secs(60),
    )
    .attach(&c);

    assert!(
        approval_gate(&c, &tool.meta(), StubTool::risk_of(&tool).as_ref(), &args,).is_none(),
        "绑定一致的 grant 应放行"
    );
    assert_eq!(
        c.get_raw(TRUSTED_APPROVED)
            .and_then(|value| value.downcast::<String>().ok())
            .as_deref()
            .map(ToString::to_string),
        Some("true".to_string()),
        "只有 grant 消费成功后才能发布内层可信标记"
    );
    assert!(
        approval_gate(&c, &tool.meta(), StubTool::risk_of(&tool).as_ref(), &args,).is_some(),
        "同一 grant 重放必须再次停等"
    );
    assert_eq!(
        c.get_raw(TRUSTED_APPROVED)
            .and_then(|value| value.downcast::<String>().ok())
            .as_deref()
            .map(ToString::to_string),
        Some("false".to_string()),
        "重放不能留下上一轮的可信标记"
    );
    assert_eq!(sandbox.decide_calls.load(Ordering::SeqCst), 2);
}

/// grant 的 call/tool/args/session/profile binding 任一变化都不能放行。
#[test]
fn grant_is_bound_to_call_tool_args_session_and_policy_binding() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let original = json!({"path": "tmp/x"});
    let cases = [
        (
            "other-call",
            "vdfs_delete",
            original.clone(),
            "session-1",
            PolicyBinding::new("default", 7),
        ),
        (
            "call-1",
            "other_tool",
            original.clone(),
            "session-1",
            PolicyBinding::new("default", 7),
        ),
        (
            "call-1",
            "vdfs_delete",
            json!({"path": "tmp/y"}),
            "session-1",
            PolicyBinding::new("default", 7),
        ),
        (
            "call-1",
            "vdfs_delete",
            original.clone(),
            "other-session",
            PolicyBinding::new("default", 7),
        ),
        (
            "call-1",
            "vdfs_delete",
            original.clone(),
            "session-1",
            PolicyBinding::new("strict", 7),
        ),
        (
            "call-1",
            "vdfs_delete",
            original.clone(),
            "session-1",
            PolicyBinding::new("default", 6),
        ),
    ];
    for (call_id, granted_tool, args, session_id, binding) in cases {
        let (c, _) = ctx_with_sandbox(SandboxDecision {
            permitted: true,
            profile_version: 7,
            ..Default::default()
        });
        bind_call(&c, "call-1", "session-1");
        ApprovalGrant::issue(
            call_id,
            granted_tool,
            &original,
            Some(session_id.into()),
            Some(binding.clone()),
            Duration::from_secs(60),
        )
        .attach(&c);
        assert!(
            approval_gate(
                &c,
                &tool.meta(),
                StubTool::risk_of(&tool).as_ref(),
                &args,
            )
            .is_some(),
            "grant 绑定变化必须失效: call={call_id}, tool={granted_tool}, session={session_id}, binding={binding:?}"
        );
    }
}

#[tokio::test]
async fn expired_grant_cannot_pass() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    bind_call(&c, "call-1", "session-1");
    let args = json!({"path": "tmp/x"});
    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &args,
        Some("session-1".into()),
        None,
        Duration::from_millis(1),
    )
    .attach(&c);
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(
        approval_gate(&c, &tool.meta(), StubTool::risk_of(&tool).as_ref(), &args,).is_some(),
        "过期 grant 必须失效"
    );
}

/// 沙箱只读一次 decide；规则风险是下限，审批模式/阈值取新裁决而非旧 ctx。
#[test]
fn sandbox_decision_is_one_snapshot_and_rule_risk_is_a_floor() {
    let tool = StubTool::new("read_file", CapabilityRiskLevel::Low);
    let (c, sandbox) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        risk: Some("high".into()),
        approval_mode: Some("interactive".into()),
        risk_threshold: Some("low".into()),
        ..Default::default()
    });
    c.set(RISK_LEVEL, "high".to_string());
    c.set(MODE, "auto".to_string());
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({}),
    )
    .expect("规则 High > 新阈值 Low，必须停等");
    assert_eq!(
        payload["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL),
        "新裁决 interactive 必须覆盖 ctx 的 stale auto"
    );
    assert_eq!(payload["prompt"]["risk_level"].as_str(), Some("high"));
    assert_eq!(sandbox.decide_calls.load(Ordering::SeqCst), 1);

    let high_tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let (c, sandbox) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        risk: Some("low".into()),
        risk_threshold: Some("high".into()),
        ..Default::default()
    });
    assert!(
        approval_gate(
            &c,
            &high_tool.meta(),
            StubTool::risk_of(&high_tool).as_ref(),
            &json!({}),
        )
        .is_none(),
        "规则 Low 不能把工具 High 调低"
    );
    assert_eq!(sandbox.decide_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn sandbox_deny_and_intercept_are_honored_before_grant_consumption() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let args = json!({"path": "tmp/x"});
    let (c, _) = ctx_with_sandbox(SandboxDecision::denied("policy says no"));
    bind_call(&c, "call-1", "session-1");
    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &args,
        Some("session-1".into()),
        None,
        Duration::from_secs(60),
    )
    .attach(&c);
    let denied = approval_gate(&c, &tool.meta(), StubTool::risk_of(&tool).as_ref(), &args)
        .expect("deny 必须拒绝");
    assert_eq!(
        denied["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED)
    );
    assert!(
        ApprovalGrant::take(
            c.as_ref(),
            "call-1",
            "vdfs_delete",
            &args,
            Some("session-1"),
            None,
        ),
        "沙箱 deny 必须先于 grant 消费"
    );
    assert!(c.get_raw(TRUSTED_APPROVED).is_none(), "deny 不应发布标记");

    let intercept = json!({"failure_kind": "needs_approval", "prompt": {"kind": "confirm"}});
    let (c, sandbox) = ctx_with_sandbox(SandboxDecision {
        permitted: false,
        intercept: Some(intercept.clone()),
        ..Default::default()
    });
    assert_eq!(
        approval_gate(&c, &tool.meta(), StubTool::risk_of(&tool).as_ref(), &args,),
        Some(intercept)
    );
    assert_eq!(sandbox.decide_calls.load(Ordering::SeqCst), 1);
}

/// 会话阈值 `high` 抬高 ⇒ High 风险也放行（`>` 是严格大于）。
#[test]
fn session_threshold_high_allows_high_risk_tool() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(RISK_LEVEL, "high".to_string());
    assert!(approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({})
    )
    .is_none());
}

/// 会话阈值 `low` 降低 ⇒ Medium 风险也要审批（新阈值生效即改变行为——本 ADR 覆盖点）。
#[test]
fn session_threshold_low_gates_medium_risk_tool() {
    let tool = StubTool::new("write_file", CapabilityRiskLevel::Medium);
    let c = ctx();
    c.set(RISK_LEVEL, "low".to_string());
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({}),
    );
    assert!(payload.is_some(), "Medium > Low 应拦下");
}

/// `mode=auto` ⇒ 不产卡；返回 `PERMISSION_DENIED` 友好错误（无人可授权时反复重试没有意义）。
#[test]
fn auto_mode_returns_permission_denied_not_confirm_card() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(MODE, "auto".to_string());
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({}),
    )
    .expect("auto 模式仍需给出失败载荷");
    assert_eq!(
        payload["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED),
    );
    assert!(
        payload.get("prompt").is_none(),
        "auto 模式不该产卡——没有用户可以批"
    );
    let msg = payload["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("请勿反复重试") || msg.contains("切换"),
        "文案要教模型不要循环重试：{msg}"
    );
}

/// 风险回答覆写生效（shell 按命令分级的机制面）——闸门读动态风险，
/// 不看工具名；否则「声明 Medium 但按高危命令升到 High」这类工具会漏拦。
#[test]
fn dynamic_risk_from_risk_fn_is_what_gate_uses() {
    // meta.risk = Medium；风险回答覆写返回 High
    let tool = StubTool::new("shell", CapabilityRiskLevel::Medium)
        .with_dyn_risk(CapabilityRiskLevel::High);
    let c = ctx();
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"command": "rm -rf /"}),
    )
    .expect("风险覆写后应按 High 拦下");
    assert_eq!(
        payload["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL)
    );
}

/// **第三档「永久放行」**：该工具的授权上限放行，且只对该工具生效。
///
/// 与会话阈值两格对照：阈值是全局的，授权是逐工具的——一个把授权当成
/// 第二个全局阈值的实现能过前半格，却会让另一个工具同档调用也放行。
#[test]
fn per_tool_grant_ceiling_allows_only_that_tool() {
    let c = ctx();
    c.set(APPROVAL_GRANTS, json!({"shell": "high"}).to_string());

    let shell = StubTool::new("shell", CapabilityRiskLevel::High);
    assert!(
        approval_gate(
            &c,
            &shell.meta(),
            StubTool::risk_of(&shell).as_ref(),
            &json!({})
        )
        .is_none(),
        "本次风险 ≤ 该工具上限 ⇒ 放行"
    );

    let other = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    assert!(
        approval_gate(
            &c,
            &other.meta(),
            StubTool::risk_of(&other).as_ref(),
            &json!({})
        )
        .is_some(),
        "授权只管自己那一格，别的工具不继承"
    );
}

/// 上限低于本次风险 ⇒ 仍然审批：授权是上限，不是「这个工具从此不问」。
#[test]
fn grant_ceiling_below_the_call_risk_still_asks() {
    let tool = StubTool::new("shell", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(APPROVAL_GRANTS, json!({"shell": "medium"}).to_string());
    let payload = approval_gate(
        &c,
        &tool.meta(),
        StubTool::risk_of(&tool).as_ref(),
        &json!({"command": "rm -rf /"}),
    )
    .expect("High > 上限 Medium ⇒ 仍要审批");
    assert_eq!(
        payload["prompt"]["risk_level"].as_str(),
        Some("high"),
        "卡上的风险档是调用自己的（前端据此决定「永久放行」抬到哪一档）"
    );
}

// ==================== 端口执行体：GatedExecutor ====================

/// 放行路径：有效 typed grant 透传到内层；`executed` 置位，结果按
/// `RESULT_KEY` 交回。模型参数里的 `approved` 不会被注入或信任。
#[tokio::test]
async fn executor_executes_inner_with_a_valid_grant() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let (c, _) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        ..Default::default()
    });
    bind_call(&c, "call-1", "session-1");
    let args = json!({"path": "tmp/x"});
    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &args,
        Some("session-1".into()),
        None,
        Duration::from_secs(60),
    )
    .attach(&c);
    let out = g
        .execute(
            exec_ctx(c.clone()),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: args,
            },
        )
        .await;
    assert!(out.success(), "放行路径不该报错");
    assert_eq!(result_of(&out)["ok"], json!(true));
    assert!(
        tool.executed.load(std::sync::atomic::Ordering::SeqCst),
        "闸门放行必须落到内层"
    );
    assert_eq!(
        c.get_raw(TRUSTED_APPROVED)
            .and_then(|value| value.downcast::<String>().ok())
            .as_deref()
            .map(ToString::to_string),
        Some("true".to_string())
    );
}

/// 原始模型 `approved:true` 即使到达执行体也不能绕过高风险闸门。
#[tokio::test]
async fn executor_ignores_raw_approved_argument() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let (c, _) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        ..Default::default()
    });
    bind_call(&c, "call-1", "session-1");
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x", "approved": true}),
            },
        )
        .await;
    assert_eq!(
        result_of(&out)["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL)
    );
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

/// 拦截路径：执行体交回确认卡载荷（`failure_kind` 在结果里）、**不**触发内层执行。
#[tokio::test]
async fn executor_returns_card_without_running_inner() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let (c, _) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        ..Default::default()
    });
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert!(out.success(), "停等不是失败：载荷里带 failure_kind");
    assert_eq!(
        result_of(&out)["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL),
    );
    assert!(
        !tool.executed.load(std::sync::atomic::Ordering::SeqCst),
        "被拦下时绝不落到内层——这就是「拦下不落内层」的核心不变量"
    );
}

/// 端口执行体必须先发布沙箱策略；策略缺席时连低风险工具也不得绕过闸门。
#[tokio::test]
async fn executor_without_approval_policy_fails_closed() {
    let tool = StubTool::new("vdfs_read", CapabilityRiskLevel::Low);
    let g = gated(&tool);
    let out = g
        .execute(
            exec_ctx(ctx()),
            PortInput {
                port: "vdfs_read".to_string(),
                payload: json!({}),
            },
        )
        .await;
    assert_eq!(
        result_of(&out)["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED),
    );
    assert!(result_of(&out)["error"]
        .as_str()
        .is_some_and(|message| message.contains("审批策略不可用")));
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

struct StaticPolicy {
    evaluation: ApprovalEvaluation,
    binding: PolicyBinding,
}

impl ApprovalPolicy for StaticPolicy {
    fn evaluate(
        &self,
        _facts: &[SandboxFact],
        _tool_name: &str,
        _args: &Value,
        _tool_risk: CapabilityRiskLevel,
        _session_threshold: CapabilityRiskLevel,
    ) -> ApprovalEvaluation {
        self.evaluation.clone()
    }

    fn binding(&self) -> Option<PolicyBinding> {
        Some(self.binding.clone())
    }
}

fn ctx_with_policy(evaluation: ApprovalEvaluation) -> Arc<dyn PluginInvokeRequest> {
    let c = ctx();
    approval_policy_init_slot(&c);
    assert!(approval_policy_publish(
        &c,
        Arc::new(StaticPolicy {
            evaluation,
            binding: PolicyBinding::new("default", 3),
        }),
    ));
    c
}

#[tokio::test]
async fn executor_uses_published_policy_and_profile_binding() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let c = ctx_with_policy(ApprovalEvaluation::require_approval(
        CapabilityRiskLevel::High,
        "interactive",
        ApprovalScopes::ALL,
        Some("删除文件需要确认".into()),
        None,
    ));
    bind_call(&c, "call-1", "session-1");
    let out = g
        .execute(
            exec_ctx(c.clone()),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert_eq!(
        result_of(&out)["prompt"]["policy"],
        json!({"profile_id": "default", "revision": 3}),
    );
    assert_eq!(
        result_of(&out)["prompt"]["approval_scopes"],
        json!(["once", "session", "permanent"]),
    );
    assert_eq!(
        result_of(&out)["prompt"]["approval_reason"],
        json!("删除文件需要确认"),
    );
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));

    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &json!({"path": "tmp/x"}),
        Some("session-1".into()),
        Some(PolicyBinding::new("default", 3)),
        Duration::from_secs(60),
    )
    .attach(&c);
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert!(out.success());
    assert!(tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn auto_session_denies_instead_of_producing_a_card_for_interactive_policy() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let c = ctx_with_policy(ApprovalEvaluation::require_approval(
        CapabilityRiskLevel::High,
        "interactive",
        ApprovalScopes::ALL,
        Some("删除文件需要确认".into()),
        None,
    ));
    c.set(MODE, "auto".to_string());
    bind_call(&c, "call-1", "session-1");
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert_eq!(
        result_of(&out)["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED),
        "无人值守会话不得产出无人可批的确认卡"
    );
    assert!(result_of(&out)["prompt"].is_null());
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn policy_deny_cannot_be_overridden_by_a_valid_grant() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let c = ctx_with_policy(ApprovalEvaluation::deny("profile blocks deletes"));
    bind_call(&c, "call-1", "session-1");
    ApprovalGrant::issue(
        "call-1",
        "vdfs_delete",
        &json!({"path": "tmp/x"}),
        Some("session-1".into()),
        Some(PolicyBinding::new("default", 3)),
        Duration::from_secs(60),
    )
    .attach(&c);
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert_eq!(
        result_of(&out)["failure_kind"].as_str(),
        Some(failure_kind::PERMISSION_DENIED),
    );
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn executor_forwards_registered_approval_summary() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let inner: Arc<dyn PortExecutor> = tool.clone();
    let g = GatedExecutor::new(tool.meta(), StubTool::risk_of(&tool), inner).with_approval(
        ApprovalSummary::targeted(
            "删除文件或目录",
            ApprovalEffect::FILE_DELETE,
            "path",
            "目标路径",
        ),
    );
    let (c, _) = ctx_with_sandbox(SandboxDecision {
        permitted: true,
        ..Default::default()
    });
    let out = g
        .execute(
            exec_ctx(c),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"path": "tmp/x"}),
            },
        )
        .await;
    assert_eq!(result_of(&out)["prompt"]["approval"]["target"], "tmp/x");
    assert!(!tool.executed.load(std::sync::atomic::Ordering::SeqCst));
}

/// 没有调用方上下文 ⇒ 诚实的非重试失败（闸门连判定的材料都没有）。
#[tokio::test]
async fn executor_without_request_context_fails_nonretryable() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let mut exec = exec_ctx(ctx());
    exec.ctx = None;
    let out = g
        .execute(
            exec,
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({}),
            },
        )
        .await;
    assert!(!out.success(), "无信封无法判闸，必须显式失败");
}
