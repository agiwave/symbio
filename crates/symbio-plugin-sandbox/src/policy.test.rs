use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::collectors::{ApprovalEvaluation, ApprovalPolicy, ApprovalScopes};
use symbio_plugin_api::{CapabilityRiskLevel, SandboxFact};

use crate::engine::{SandboxProfileConfig, SandboxRegistry};
use crate::grants::GrantStore;
use crate::policy::Policy;

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    grants: Arc<GrantStore>,
}

impl Fixture {
    fn new(
        selected: Option<&str>,
        extra: Vec<(&str, SandboxProfileConfig)>,
    ) -> (Self, Arc<dyn ApprovalPolicy>) {
        let root = std::env::temp_dir().join(format!(
            "sandbox-policy-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let mut profiles = vec![(
            "default".to_string(),
            SandboxProfileConfig {
                name: "默认".into(),
                approval_mode: Some("interactive".into()),
                risk_threshold: Some("medium".into()),
                ..Default::default()
            },
        )];
        profiles.extend(
            extra
                .into_iter()
                .map(|(id, config)| (id.to_string(), config)),
        );
        let registry = Arc::new(std::sync::RwLock::new(
            SandboxRegistry::try_from_profiles(profiles).expect("valid profiles"),
        ));
        let grants = Arc::new(GrantStore::load(
            &root.join("sandbox"),
            &root.join("session"),
        ));
        let policy = Policy::create(
            registry.clone(),
            grants.clone(),
            selected.map(str::to_string),
        );
        (Self { root, grants }, policy)
    }

    fn evaluate(
        policy: &dyn ApprovalPolicy,
        tool: &str,
        risk: CapabilityRiskLevel,
        session: CapabilityRiskLevel,
    ) -> ApprovalEvaluation {
        policy.evaluate(&[], tool, &json!({}), risk, session)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn profile(name: &str, mode: &str, threshold: &str) -> SandboxProfileConfig {
    SandboxProfileConfig {
        name: name.into(),
        approval_mode: Some(mode.into()),
        risk_threshold: Some(threshold.into()),
        ..Default::default()
    }
}

#[test]
fn unselected_uses_default_and_session_preference_cannot_raise_profile_ceiling() {
    let (_fixture, policy) = Fixture::new(None, vec![]);
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::High,
    )
    .is_allowed());
    let evaluation = Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::Low,
    );
    assert!(evaluation.needs_approval());
    assert_eq!(evaluation.scopes(), ApprovalScopes::ALL);
    assert_eq!(evaluation.binding().unwrap().profile_id(), "default");
    assert!(evaluation
        .reason()
        .unwrap()
        .contains("高于本会话自动放行偏好"));
    assert!(!evaluation.reason().unwrap().contains("高于当前沙箱"));
    assert_eq!(
        Fixture::evaluate(
            policy.as_ref(),
            "tool",
            CapabilityRiskLevel::High,
            CapabilityRiskLevel::High,
        )
        .scopes(),
        ApprovalScopes::ONCE
    );
}

#[test]
fn strict_and_unattended_narrow_automatic_and_approval_scopes() {
    let mut strict = profile("严格", "interactive", "low");
    strict.allow_permanent_approval = Some(false);
    let mut unattended = profile("无人值守", "auto", "medium");
    unattended.allow_session_approval = Some(false);
    unattended.allow_permanent_approval = Some(false);
    let (_fixture, policy) = Fixture::new(
        Some("strict"),
        vec![("strict", strict), ("unattended", unattended)],
    );

    let medium = Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::High,
    );
    assert!(medium.needs_approval());
    assert_eq!(medium.scopes(), ApprovalScopes::ONCE);
    let high = Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::High,
        CapabilityRiskLevel::High,
    );
    assert_eq!(high.scopes(), ApprovalScopes::ONCE);

    let (_fixture, policy) = Fixture::new(
        Some("unattended"),
        vec![("unattended", profile("无人值守", "auto", "medium"))],
    );
    let evaluation = Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::High,
        CapabilityRiskLevel::High,
    );
    assert!(evaluation.needs_approval());
    assert_eq!(evaluation.mode(), Some("auto"));
    assert_eq!(evaluation.scopes(), ApprovalScopes::ONCE);
}

#[test]
fn disabled_session_approval_ignores_lower_session_preference() {
    let mut custom = profile("仅档案阈值", "interactive", "medium");
    custom.allow_session_approval = Some(false);
    let (_fixture, policy) = Fixture::new(Some("custom"), vec![("custom", custom)]);
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::Low,
    )
    .is_allowed());
}

#[test]
fn explicit_none_is_unrestricted_but_missing_profile_denies() {
    let (_fixture, policy) = Fixture::new(
        Some("none"),
        vec![(
            "none",
            SandboxProfileConfig {
                approval_mode: Some("auto".into()),
                risk_threshold: Some("high".into()),
                allow_session_approval: Some(false),
                allow_permanent_approval: Some(false),
                ..Default::default()
            },
        )],
    );
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::High,
        CapabilityRiskLevel::Low,
    )
    .is_allowed());

    let (_fixture, policy) = Fixture::new(Some("missing"), vec![]);
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::Low,
        CapabilityRiskLevel::High,
    )
    .is_denied());
}

#[test]
fn dynamic_rule_risk_is_a_floor_and_hard_deny_wins() {
    let mut custom = profile("自定义", "interactive", "medium");
    custom.rules.push(crate::engine::SandboxRule {
        kind: "shell_command".into(),
        mode: crate::engine::RuleMatch::Contains,
        value: "rm -rf".into(),
        action: crate::engine::RuleAction::Allow,
        risk: crate::engine::RuleRisk::High,
    });
    let (_fixture, policy) = Fixture::new(Some("custom"), vec![("custom", custom)]);
    let facts = [SandboxFact::new("shell_command", "rm -rf tmp")];
    let evaluation = policy.evaluate(
        &facts,
        "shell",
        &json!({"command": "rm -rf tmp"}),
        CapabilityRiskLevel::Low,
        CapabilityRiskLevel::High,
    );
    assert!(evaluation.needs_approval());
    assert_eq!(evaluation.risk(), Some(CapabilityRiskLevel::High));
    assert_eq!(evaluation.scopes(), ApprovalScopes::ONCE);

    let mut denied = profile("拒绝", "interactive", "high");
    denied.rules.push(crate::engine::SandboxRule {
        kind: "shell_command".into(),
        mode: crate::engine::RuleMatch::Contains,
        value: "rm".into(),
        action: crate::engine::RuleAction::Deny,
        risk: crate::engine::RuleRisk::Inherit,
    });
    let (_fixture, policy) = Fixture::new(Some("denied"), vec![("denied", denied)]);
    assert!(policy
        .evaluate(
            &facts,
            "shell",
            &json!({}),
            CapabilityRiskLevel::Low,
            CapabilityRiskLevel::High,
        )
        .is_denied());
}

#[test]
fn permanent_grants_are_profile_scoped_and_capped_by_profile_threshold() {
    let (_fixture, policy) = Fixture::new(None, vec![]);
    _fixture
        .grants
        .grant("default", "shell", CapabilityRiskLevel::High)
        .unwrap();
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "shell",
        CapabilityRiskLevel::High,
        CapabilityRiskLevel::Low,
    )
    .needs_approval());
    _fixture
        .grants
        .grant("default", "shell", CapabilityRiskLevel::Medium)
        .unwrap();
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "shell",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::Low,
    )
    .is_allowed());
    assert!(Fixture::evaluate(
        policy.as_ref(),
        "other",
        CapabilityRiskLevel::Medium,
        CapabilityRiskLevel::Low,
    )
    .needs_approval());
}

#[test]
fn profile_revisions_isolate_unrelated_changes_and_prevent_reuse_after_delete() {
    fn snapshot(registry: &SandboxRegistry) -> Vec<(String, SandboxProfileConfig)> {
        registry
            .archives()
            .iter()
            .map(|(id, config)| (id.clone(), config.clone()))
            .collect()
    }

    let mut registry = SandboxRegistry::try_from_profiles(vec![
        ("default".into(), profile("默认", "interactive", "medium")),
        ("other".into(), profile("其他", "interactive", "medium")),
    ])
    .unwrap();
    registry.replace(SandboxRegistry::try_from_profiles(snapshot(&registry)).unwrap());
    let default_before = registry.policy_binding("default");
    let other_before = registry.policy_binding("other");

    let mut changed = snapshot(&registry);
    changed
        .iter_mut()
        .find(|(id, _)| id == "other")
        .unwrap()
        .1
        .risk_threshold = Some("high".into());
    registry.replace(SandboxRegistry::try_from_profiles(changed).unwrap());
    assert_eq!(registry.policy_binding("default"), default_before);
    assert_ne!(registry.policy_binding("other"), other_before);

    let before_delete = registry.policy_binding("other");
    let removed = snapshot(&registry)
        .into_iter()
        .filter(|(id, _)| id != "other")
        .collect();
    registry.replace(SandboxRegistry::try_from_profiles(removed).unwrap());
    let mut readded = snapshot(&registry);
    readded.push(("other".into(), profile("其他", "interactive", "medium")));
    registry.replace(SandboxRegistry::try_from_profiles(readded).unwrap());
    assert_ne!(registry.policy_binding("other"), before_delete);
}

#[test]
fn intercept_is_returned_before_any_automatic_approval() {
    let mut custom = profile("拦截", "interactive", "high");
    custom
        .blocked_tools
        .insert("tool".into(), json!({"intercepted": true}));
    let (_fixture, policy) = Fixture::new(Some("custom"), vec![("custom", custom)]);
    let evaluation = Fixture::evaluate(
        policy.as_ref(),
        "tool",
        CapabilityRiskLevel::High,
        CapabilityRiskLevel::High,
    );
    assert_eq!(evaluation.intercept(), Some(&json!({"intercepted": true})));
}
