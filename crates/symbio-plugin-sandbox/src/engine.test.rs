//! `engine.rs` 的单元测试 —— 沙箱引擎的裁决语义（D-531 / DE-058）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。钉住两件事：
//! 无选中沙箱时全放行（诚实降级），以及显式配置下的白名单 / 拦截裁决。

use super::*;

fn engine_with(profile: SandboxProfile) -> Engine {
    let mut reg = SandboxRegistry::default();
    reg.install("test", profile);
    Engine::new(
        Arc::new(std::sync::RwLock::new(reg)),
        Some("test".to_string()),
    )
}

#[test]
fn no_selected_sandbox_permits_everything() {
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::default())),
        None,
    );
    assert!(engine.permits(&[SandboxFact::new("shell_command", "rm -rf /")]));
    assert!(engine
        .intercept("any_tool", &serde_json::json!({}))
        .is_none());
}

#[test]
fn a_configured_kind_denies_values_not_in_whitelist() {
    let mut profile = SandboxProfile::default();
    profile.fact_rules.insert(
        "shell_command".to_string(),
        FactRule {
            allowed_values: vec!["git status".to_string()],
        },
    );
    let engine = engine_with(profile);
    assert!(engine.permits(&[SandboxFact::new("shell_command", "git status")]));
    assert!(!engine.permits(&[SandboxFact::new("shell_command", "rm -rf /")]));
    // 没配规则的 kind 不设限。
    assert!(engine.permits(&[SandboxFact::new("url", "http://example.com")]));
}

#[test]
fn intercept_only_blocks_explicitly_configured_tools() {
    let mut profile = SandboxProfile::default();
    profile.blocked_tools.insert(
        "dangerous_tool".to_string(),
        serde_json::json!({ "blocked": true }),
    );
    let engine = engine_with(profile);
    assert!(engine
        .intercept("dangerous_tool", &serde_json::json!({}))
        .is_some());
    assert!(engine
        .intercept("ordinary_tool", &serde_json::json!({}))
        .is_none());
}

#[test]
fn all_facts_must_pass() {
    let mut profile = SandboxProfile::default();
    profile.fact_rules.insert(
        "shell_command".to_string(),
        FactRule {
            allowed_values: vec!["ls".to_string()],
        },
    );
    let engine = engine_with(profile);
    // 两条事实里有一条不在白名单 ⇒ 整体拒绝。
    assert!(!engine.permits(&[
        SandboxFact::new("shell_command", "ls"),
        SandboxFact::new("shell_command", "rm -rf /"),
    ]));
}

#[test]
fn config_builds_a_live_registry() {
    let config: SandboxConfig = serde_json::from_value(serde_json::json!({
        "sandboxes": {
            "strict": {
                "name": "严格",
                "allowed_facts": { "shell_command": ["git status"] },
                "blocked_tools": { "dangerous_tool": { "blocked": true } }
            }
        },
        "active": "strict"
    }))
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::from_config(config))),
        Some("strict".to_string()),
    );
    assert!(engine.permits(&[SandboxFact::new("shell_command", "git status")]));
    assert!(!engine.permits(&[SandboxFact::new("shell_command", "rm -rf /")]));
    assert!(engine
        .intercept("dangerous_tool", &serde_json::json!({}))
        .is_some());
}

#[test]
fn approval_params_flow_from_selected_archive() {
    // 承接 mode/risk_level 的审批开关随选中档案读出，缺席 = None（装配侧回落默认）。
    let profiles = vec![(
        "strict".to_string(),
        SandboxProfileConfig {
            name: "严格".into(),
            approval_mode: Some("auto".into()),
            risk_threshold: Some("high".into()),
            ..Default::default()
        },
    )];
    let reg = SandboxRegistry::from_profiles(profiles);
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(reg)),
        Some("strict".to_string()),
    );
    assert_eq!(
        engine.approval_params(),
        (Some("auto".to_string()), Some("high".to_string()))
    );
    // 选中一个不存在的档案 ⇒ 全放行降级（装配侧回落默认）。
    let empty = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::default())),
        Some("missing".to_string()),
    );
    assert_eq!(empty.approval_params(), (None, None));
}
