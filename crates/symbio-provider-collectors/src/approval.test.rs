//! `providers/collectors/approval.rs` 的单元测试。
//!
//! 覆盖审批闸门的两处形状：
//! 1. **判定 [`approval_gate`]**——风险档与阈值的比较、`approved: true` 放行、
//!    逐工具的永久放行上限、`mode=auto` 走 `PERMISSION_DENIED`（无人可授权时
//!    **不产卡**），其余产卡。
//! 2. **注册面投影 [`ApprovalGate`]**——`meta` / `name` / `risk_for` 原样透传；
//!    `execute` 只在闸门放行时才落到内层。这条是 vdfs / web / mcp / skill 都受益的
//!    那条唯一路径（`DefaultToolVisitor::put`）。

use super::*;
use serde_json::json;
use symbio_core::{failure_kind, ExecAbortSignal, ExecEventSink, PluginSimpleRequest};

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
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        self.executed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"ok": true}))
    }
}

/// `ExecEnv` 的最小构造（本模块只用它做参数透传）——从 plugin 现有测试借来。
fn env_stub() -> ExecEnv {
    ExecEnv::new(ExecEventSink::silent(), ExecAbortSignal::new())
}

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

// ==================== 判定：approval_gate ====================

/// 阈值缺席（默认 Medium）+ `risk = High` ⇒ 需要审批，产 `NEEDS_APPROVAL` 卡。
#[test]
fn high_risk_over_default_threshold_produces_confirm_card() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    let payload = approval_gate(&c, &*tool, &json!({"path": "tmp/x"}))
        .unwrap()
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
        approval_gate(&c, &*tool, &json!({})).unwrap().is_none(),
        "Low ≤ Medium(默认) 应放行"
    );
}

/// `approved: true` 覆盖判据——用户已在卡上批过，二次进入直接放行。
/// 这是 t27 审批恢复链路的工作方式：编排层重放原调用时把 `approved` 塞进参数。
#[test]
fn approved_argument_bypasses_gate_even_for_high_risk() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    assert!(approval_gate(&c, &*tool, &json!({"approved": true}))
        .unwrap()
        .is_none());
}

/// 会话阈值 `high` 抬高 ⇒ High 风险也放行（`>` 是严格大于）。
#[test]
fn session_threshold_high_allows_high_risk_tool() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(RISK_LEVEL, "high".to_string());
    assert!(approval_gate(&c, &*tool, &json!({})).unwrap().is_none());
}

/// 会话阈值 `low` 降低 ⇒ Medium 风险也要审批（新阈值生效即改变行为——本 ADR 覆盖点）。
#[test]
fn session_threshold_low_gates_medium_risk_tool() {
    let tool = StubTool::new("write_file", CapabilityRiskLevel::Medium);
    let c = ctx();
    c.set(RISK_LEVEL, "low".to_string());
    let payload = approval_gate(&c, &*tool, &json!({})).unwrap();
    assert!(payload.is_some(), "Medium > Low 应拦下");
}

/// `mode=auto` ⇒ 不产卡；返回 `PERMISSION_DENIED` 友好错误（无人可授权时反复重试没有意义）。
#[test]
fn auto_mode_returns_permission_denied_not_confirm_card() {
    let tool = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(MODE, "auto".to_string());
    let payload = approval_gate(&c, &*tool, &json!({}))
        .unwrap()
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

/// `risk_for` 覆写生效（shell 按命令分级的机制面）——闸门读工具动态风险，
/// 不看工具名；否则「声明 Medium 但按高危命令升到 High」这类工具会漏拦。
#[test]
fn dynamic_risk_from_risk_for_is_what_gate_uses() {
    // meta.risk = Medium；risk_for 覆写返回 High
    let tool = StubTool::new("shell", CapabilityRiskLevel::Medium)
        .with_dyn_risk(CapabilityRiskLevel::High);
    let c = ctx();
    let payload = approval_gate(&c, &*tool, &json!({"command": "rm -rf /"}))
        .unwrap()
        .expect("risk_for 覆写后应按 High 拦下");
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
        approval_gate(&c, &*shell, &json!({})).unwrap().is_none(),
        "本次风险 ≤ 该工具上限 ⇒ 放行"
    );

    let other = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    assert!(
        approval_gate(&c, &*other, &json!({})).unwrap().is_some(),
        "授权只管自己那一格，别的工具不继承"
    );
}

/// 上限低于本次风险 ⇒ 仍然审批：授权是上限，不是「这个工具从此不问」。
#[test]
fn grant_ceiling_below_the_call_risk_still_asks() {
    let tool = StubTool::new("shell", CapabilityRiskLevel::High);
    let c = ctx();
    c.set(APPROVAL_GRANTS, json!({"shell": "medium"}).to_string());
    let payload = approval_gate(&c, &*tool, &json!({"command": "rm -rf /"}))
        .unwrap()
        .expect("High > 上限 Medium ⇒ 仍要审批");
    assert_eq!(
        payload["prompt"]["risk_level"].as_str(),
        Some("high"),
        "卡上的风险档是调用自己的（前端据此决定「永久放行」抬到哪一档）"
    );
}

// ==================== 注册面投影：ApprovalGate ====================

/// wrapper **不改** `meta` / `name` / `risk_for`——否则按名查表、目录页与
/// 分级判定都会看到包装层给的默认档，`vdfs_delete` 的 High 声明就丢了。
#[tokio::test]
async fn wrapper_preserves_meta_name_and_dynamic_risk() {
    let inner = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let gated = ApprovalGate::new(inner.clone());
    assert_eq!(gated.name(), "vdfs_delete");
    assert_eq!(gated.meta().risk, Some(CapabilityRiskLevel::High));
    assert_eq!(gated.risk_for(&json!({})), CapabilityRiskLevel::High);
}

/// 放行路径：`approved: true` 时 wrapper 落到内层执行；`executed` 置位。
#[tokio::test]
async fn wrapper_executes_inner_when_approved() {
    let inner = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let gated = ApprovalGate::new(inner.clone());
    let c = ctx();
    let out = gated
        .execute(json!({"approved": true}), &env_stub(), c)
        .await
        .unwrap();
    assert_eq!(out["ok"], json!(true));
    assert!(
        inner.executed.load(std::sync::atomic::Ordering::SeqCst),
        "闸门放行必须落到内层"
    );
}

/// 拦截路径：wrapper 返回确认卡载荷、**不**触发内层执行。
#[tokio::test]
async fn wrapper_returns_card_without_running_inner() {
    let inner = StubTool::new("vdfs_delete", CapabilityRiskLevel::High);
    let gated = ApprovalGate::new(inner.clone());
    let c = ctx();
    let out = gated
        .execute(json!({"path": "tmp/x"}), &env_stub(), c)
        .await
        .unwrap();
    assert_eq!(
        out["failure_kind"].as_str(),
        Some(failure_kind::NEEDS_APPROVAL),
    );
    assert!(
        !inner.executed.load(std::sync::atomic::Ordering::SeqCst),
        "被拦下时绝不落到内层——这就是「拦下不落内层」的核心不变量"
    );
}
