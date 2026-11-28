use std::sync::Arc;

use serde_json::Value;
use symbio_plugin_api::collectors::{
    ApprovalEvaluation, ApprovalPolicy, ApprovalScopes, PolicyBinding,
};
use symbio_plugin_api::{CapabilityRiskLevel, SandboxFact};

use crate::engine::SandboxRegistry;
use crate::grants::GrantStore;

pub(crate) struct Policy {
    registry: Arc<std::sync::RwLock<SandboxRegistry>>,
    grants: Arc<GrantStore>,
    selected: Option<String>,
}

impl Policy {
    pub(crate) fn create(
        registry: Arc<std::sync::RwLock<SandboxRegistry>>,
        grants: Arc<GrantStore>,
        selected: Option<String>,
    ) -> Arc<dyn ApprovalPolicy> {
        Arc::new(Self {
            registry,
            grants,
            selected,
        })
    }
}

impl ApprovalPolicy for Policy {
    fn evaluate(
        &self,
        facts: &[SandboxFact],
        tool_name: &str,
        args: &Value,
        tool_risk: CapabilityRiskLevel,
        session_threshold: CapabilityRiskLevel,
    ) -> ApprovalEvaluation {
        let registry = self.registry.read().unwrap_or_else(|p| p.into_inner());
        let id = match registry.resolved_profile_id(self.selected.as_deref()) {
            Ok(id) => id,
            Err(reason) => return ApprovalEvaluation::deny(reason),
        };
        let Some(config) = registry.profile_config(&id) else {
            return ApprovalEvaluation::deny(format!("sandbox profile '{id}' is unavailable"));
        };
        let binding = registry.policy_binding(&id);
        let decision = match registry.decide_policy(Some(&id), facts, tool_name, args) {
            Ok(decision) => decision,
            Err(reason) => return ApprovalEvaluation::deny(reason),
        };
        if let Some(intercept) = decision.intercept {
            return ApprovalEvaluation::intercepted(intercept);
        }
        if !decision.permitted {
            return ApprovalEvaluation::deny(
                decision
                    .reason
                    .unwrap_or_else(|| "sandbox denied the tool call".into()),
            );
        }

        let floor = decision
            .risk
            .as_deref()
            .and_then(CapabilityRiskLevel::parse)
            .unwrap_or(CapabilityRiskLevel::Low);
        let risk = tool_risk.max(floor);
        let profile_threshold = CapabilityRiskLevel::parse(config.risk_ceiling())
            .unwrap_or(CapabilityRiskLevel::Medium);
        let scopes = config.approval_scopes();
        let session_threshold = if scopes.contains(ApprovalScopes::SESSION) {
            session_threshold.min(profile_threshold)
        } else {
            profile_threshold
        };
        if risk <= session_threshold {
            return ApprovalEvaluation::allow();
        }
        if scopes.contains(ApprovalScopes::PERMANENT)
            && self
                .grants
                .list(&id)
                .capped_at(profile_threshold)
                .ceiling(tool_name)
                .is_some_and(|ceiling| risk <= ceiling)
        {
            return ApprovalEvaluation::allow();
        }

        let scopes = if risk <= profile_threshold {
            scopes
        } else {
            ApprovalScopes::ONCE
        };
        let mode = config.approval_mode.as_deref().unwrap_or("interactive");
        let reason = if risk > profile_threshold {
            format!(
                "本次风险为 {}，高于当前沙箱「{}」的自动放行上限 {}",
                risk.as_str(),
                config.name,
                profile_threshold.as_str()
            )
        } else if !scopes.contains(ApprovalScopes::SESSION) {
            format!(
                "当前沙箱「{}」未开放本会话自动放行，本次需单独审批",
                config.name
            )
        } else {
            format!(
                "本次风险为 {}，高于本会话自动放行偏好；当前沙箱「{}」的上限为 {}",
                risk.as_str(),
                config.name,
                profile_threshold.as_str()
            )
        };
        ApprovalEvaluation::require_approval(risk, mode, scopes, Some(reason), Some(binding))
    }

    fn binding(&self) -> Option<PolicyBinding> {
        let registry = self.registry.read().unwrap_or_else(|p| p.into_inner());
        let id = registry
            .resolved_profile_id(self.selected.as_deref())
            .ok()?;
        Some(registry.policy_binding(&id))
    }
}

#[cfg(test)]
#[path = "policy.test.rs"]
mod tests;
