//! `providers/collectors/approval.rs` 的单元测试。
//!
//! 覆盖审批闸门的两处形状：
//! 1. **判定 [`approval_gate`]**——风险档与阈值的比较、`approved: true` 放行、
//!    逐工具的永久放行上限、`mode=auto` 走 `PERMISSION_DENIED`（无人可授权时
//!    **不产卡**），其余产卡。
//! 2. **端口执行体 [`GatedExecutor`]**——判定是执行的第一段：拦下时停等载荷按
//!    `RESULT_KEY` 交回且**不落内层**；放行才拆信封执行。这条是 vdfs / web / mcp /
//!    skill 都受益的唯一路径（`DefaultToolVisitor::put` 派生端口行时裹上的那个）。

use super::*;
use serde_json::json;
use symbio_kernel::capability::failure_kind;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::PluginSimpleRequest;
use symbio_plugin_api::port::PortFactEmit;
use symbio_plugin_api::{PortExecContext, PortInput};

/// 最小可判定的能力桩：固定风险、可选动态覆写、记录是否被真正执行。
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
}

#[async_trait]
impl Capability for StubTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.name.to_string(),
            description: format!("{} 的说明", self.name),
            input_schema: json!({"type":"object"}),
            risk: Some(self.risk),
            ..Default::default()
        }
    }
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn risk_for(&self, _args: &Value) -> CapabilityRiskLevel {
        self.dyn_risk.unwrap_or(self.risk)
    }
    async fn execute(
        &self,
        _args: Value,
        _env: &symbio_kernel::exec::ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, symbio_kernel::plugin::PluginError> {
        self.executed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"ok": true}))
    }
}

/// 判定用：从桩取风险回答（与 put 收口同一个形状——闭包捕获能力，动态覆写生效）。
fn risk_of(tool: &Arc<StubTool>) -> RiskOf {
    let t = Arc::clone(tool);
    Arc::new(move |args: &Value| t.risk_for(args))
}

/// 端口执行体用：同一形状裹出 `GatedExecutor`。
fn gated(tool: &Arc<StubTool>) -> Arc<GatedExecutor> {
    let cap: Arc<dyn Capability> = tool.clone();
    Arc::new(GatedExecutor::new(tool.meta(), risk_of(tool), cap))
}

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
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
        group: String::new(),
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
        risk_of(&tool).as_ref(),
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

/// 阈值缺席 + `risk = Low` ⇒ 放行。
#[test]
fn low_risk_passes_through_default_threshold() {
    let tool = StubTool::new("read_file", CapabilityRiskLevel::Low);
    let c = ctx();
    assert!(
        approval_gate(&c, &tool.meta(), risk_of(&tool).as_ref(), &json!({})).is_none(),
        "Low ≤ Medium(默认) 应放行"
    );
}

/// `approved: true` 覆盖判据——用户已在卡上批过，二次进入直接放行。
/// 这是 t27 审批恢复链路的工作方式：编排层重放原调用时把 `approved` 塞进参数。
#[test]
fn approved_argument_bypasses_gate_even_for_high_risk() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    assert!(approval_gate(
        &c,
        &tool.meta(),
        risk_of(&tool).as_ref(),
        &json!({"approved": true})
    )
    .is_none());
}

/// 会话阈值 `high` 抬高 ⇒ High 风险也放行（`>` 是严格大于）。
#[test]
fn session_threshold_high_allows_high_risk_tool() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(RISK_LEVEL, "high".to_string());
    assert!(approval_gate(&c, &tool.meta(), risk_of(&tool).as_ref(), &json!({})).is_none());
}

/// 会话阈值 `low` 降低 ⇒ Medium 风险也要审批（新阈值生效即改变行为——本 ADR 覆盖点）。
#[test]
fn session_threshold_low_gates_medium_risk_tool() {
    let tool = StubTool::new("write_file", CapabilityRiskLevel::Medium);
    let c = ctx();
    c.set(RISK_LEVEL, "low".to_string());
    let payload = approval_gate(&c, &tool.meta(), risk_of(&tool).as_ref(), &json!({}));
    assert!(payload.is_some(), "Medium > Low 应拦下");
}

/// `mode=auto` ⇒ 不产卡；返回 `PERMISSION_DENIED` 友好错误（无人可授权时反复重试没有意义）。
#[test]
fn auto_mode_returns_permission_denied_not_confirm_card() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(MODE, "auto".to_string());
    let payload = approval_gate(&c, &tool.meta(), risk_of(&tool).as_ref(), &json!({}))
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
        risk_of(&tool).as_ref(),
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
        approval_gate(&c, &shell.meta(), risk_of(&shell).as_ref(), &json!({})).is_none(),
        "本次风险 ≤ 该工具上限 ⇒ 放行"
    );

    let other = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    assert!(
        approval_gate(&c, &other.meta(), risk_of(&other).as_ref(), &json!({})).is_some(),
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
        risk_of(&tool).as_ref(),
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

/// 放行路径：`approved: true` 时执行体落到内层；`executed` 置位，结果按
/// `RESULT_KEY` 交回。
#[tokio::test]
async fn executor_executes_inner_when_approved() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let out = g
        .execute(
            exec_ctx(ctx()),
            PortInput {
                port: "vdfs_delete".to_string(),
                payload: json!({"approved": true}),
            },
        )
        .await;
    assert!(out.success(), "放行路径不该报错");
    assert_eq!(result_of(&out)["ok"], json!(true));
    assert!(
        tool.executed.load(std::sync::atomic::Ordering::SeqCst),
        "闸门放行必须落到内层"
    );
}

/// 拦截路径：执行体交回确认卡载荷（`failure_kind` 在结果里）、**不**触发内层执行。
#[tokio::test]
async fn executor_returns_card_without_running_inner() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let g = gated(&tool);
    let out = g
        .execute(
            exec_ctx(ctx()),
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
    assert!(!out.success(), "无信封无法拆参，必须显式失败");
}
