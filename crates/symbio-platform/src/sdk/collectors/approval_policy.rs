//! 审批策略协议：sandbox 实现判断，平台闸门负责强制执行。

use std::sync::{Arc, OnceLock};

use crate::kernel::capability::CapabilityRiskLevel;
use crate::kernel::keys::SymbioKey;
use crate::kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};
use crate::sdk::sandbox::SandboxFact;
use serde_json::Value;

pub use crate::sdk::capability::PolicyBinding;

/// 沙箱对一次工具调用的审批策略判断。
pub trait ApprovalPolicy: Send + Sync {
    fn evaluate(
        &self,
        facts: &[SandboxFact],
        tool_name: &str,
        args: &Value,
        tool_risk: CapabilityRiskLevel,
        session_threshold: CapabilityRiskLevel,
    ) -> ApprovalEvaluation;

    /// 当前选中档案的策略绑定；批准恢复只比较服务端快照身份。
    fn binding(&self) -> Option<PolicyBinding> {
        None
    }
}

/// 不透明策略结果；私有表示避免扩大 SDK 公共数据面。
#[derive(Debug, Clone)]
pub struct ApprovalEvaluation(ApprovalEvaluationInner);

#[derive(Debug, Clone)]
enum ApprovalEvaluationInner {
    Allow,
    Deny(String),
    Intercept(Value),
    NeedsApproval(NeedsApproval),
}

#[derive(Debug, Clone)]
struct NeedsApproval {
    risk: CapabilityRiskLevel,
    mode: String,
    scopes: ApprovalScopes,
    reason: Option<String>,
    binding: Option<PolicyBinding>,
}

impl ApprovalEvaluation {
    pub fn allow() -> Self {
        Self(ApprovalEvaluationInner::Allow)
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self(ApprovalEvaluationInner::Deny(reason.into()))
    }

    pub fn intercepted(payload: Value) -> Self {
        Self(ApprovalEvaluationInner::Intercept(payload))
    }

    pub fn require_approval(
        risk: CapabilityRiskLevel,
        mode: impl Into<String>,
        scopes: ApprovalScopes,
        reason: Option<String>,
        binding: Option<PolicyBinding>,
    ) -> Self {
        Self(ApprovalEvaluationInner::NeedsApproval(NeedsApproval {
            risk,
            mode: mode.into(),
            scopes,
            reason,
            binding,
        }))
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self.0, ApprovalEvaluationInner::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self.0, ApprovalEvaluationInner::Deny(_))
    }

    pub fn needs_approval(&self) -> bool {
        matches!(self.0, ApprovalEvaluationInner::NeedsApproval(_))
    }

    pub fn intercept(&self) -> Option<&Value> {
        match &self.0 {
            ApprovalEvaluationInner::Intercept(payload) => Some(payload),
            _ => None,
        }
    }

    pub fn risk(&self) -> Option<CapabilityRiskLevel> {
        match &self.0 {
            ApprovalEvaluationInner::NeedsApproval(spec) => Some(spec.risk),
            _ => None,
        }
    }

    pub fn mode(&self) -> Option<&str> {
        match &self.0 {
            ApprovalEvaluationInner::NeedsApproval(spec) => Some(&spec.mode),
            _ => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match &self.0 {
            ApprovalEvaluationInner::Deny(reason) => Some(reason),
            ApprovalEvaluationInner::NeedsApproval(spec) => spec.reason.as_deref(),
            _ => None,
        }
    }

    pub fn scopes(&self) -> ApprovalScopes {
        match &self.0 {
            ApprovalEvaluationInner::NeedsApproval(spec) => spec.scopes,
            _ => ApprovalScopes::ONCE,
        }
    }

    pub fn binding(&self) -> Option<&PolicyBinding> {
        match &self.0 {
            ApprovalEvaluationInner::NeedsApproval(spec) => spec.binding.as_ref(),
            _ => None,
        }
    }
}

/// 审批卡允许的授权范围。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApprovalScopes(u8);

impl ApprovalScopes {
    pub const ONCE: Self = Self(1 << 0);
    pub const SESSION: Self = Self(1 << 1);
    pub const PERMANENT: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::ONCE.0 | Self::SESSION.0 | Self::PERMANENT.0);

    pub const fn contains(self, scope: Self) -> bool {
        self.0 & scope.0 != 0
    }

    pub const fn with(self, scope: Self) -> Self {
        Self(self.0 | scope.0)
    }

    pub fn names(self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.contains(Self::ONCE) {
            names.push("once");
        }
        if self.contains(Self::SESSION) {
            names.push("session");
        }
        if self.contains(Self::PERMANENT) {
            names.push("permanent");
        }
        names
    }
}

/// 审批策略共享槽；与 SandboxFace 同形，但只发布判断，不发布执行能力。
pub struct ApprovalPolicyKey;
impl SymbioKey for ApprovalPolicyKey {
    type Value = Arc<OnceLock<Arc<dyn ApprovalPolicy>>>;
    const WIRE: bool = false;

    fn name(&self) -> &'static str {
        "approval_policy"
    }

    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }

    fn format(&self, _v: &Self::Value) -> String {
        self.name().to_string()
    }
}
pub const APPROVAL_POLICY: ApprovalPolicyKey = ApprovalPolicyKey;

pub fn approval_policy_init_slot(ctx: &Arc<dyn PluginInvokeRequest>) {
    if ctx.get(APPROVAL_POLICY).is_none() {
        ctx.set(APPROVAL_POLICY, Arc::new(OnceLock::new()));
    }
}

pub fn approval_policy_publish(
    ctx: &Arc<dyn PluginInvokeRequest>,
    policy: Arc<dyn ApprovalPolicy>,
) -> bool {
    approval_policy_init_slot(ctx);
    ctx.get(APPROVAL_POLICY)
        .and_then(|slot| slot.set(policy).ok())
        .is_some()
}

pub fn approval_policy_of(ctx: &dyn PluginInvokeRequest) -> Option<Arc<dyn ApprovalPolicy>> {
    ctx.get(APPROVAL_POLICY)
        .and_then(|slot| slot.get().cloned())
}
