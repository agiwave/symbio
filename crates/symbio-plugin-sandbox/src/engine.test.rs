//! `engine.rs` 的单元测试 —— 沙箱规则裁决语义（D-531 / DE-058）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。钉住四件事：
//! 无选中沙箱全放行、有序规则首个命中、四种匹配模式、风险档与旧档迁移。

use super::*;

fn engine_with(profile: SandboxProfile) -> Engine {
    let mut reg = SandboxRegistry::default();
    reg.install("test", profile);
    Engine::new(
        Arc::new(std::sync::RwLock::new(reg)),
        Some("test".to_string()),
    )
}

fn rule(
    kind: &str,
    mode: RuleMatch,
    value: &str,
    action: RuleAction,
    risk: RuleRisk,
) -> SandboxRule {
    SandboxRule {
        kind: kind.into(),
        mode,
        value: value.into(),
        action,
        risk,
    }
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
    assert_eq!(
        engine.risk_of(&[SandboxFact::new("shell_command", "x")]),
        None
    );
}

#[test]
fn first_matching_rule_wins_deny_blocks() {
    let profile = SandboxProfile {
        rules: vec![
            // 先拒绝危险命令，再允许全部命令 —— 顺序不能反。
            rule(
                "shell_command",
                RuleMatch::Contains,
                "rm -rf",
                RuleAction::Deny,
                RuleRisk::High,
            ),
            rule(
                "shell_command",
                RuleMatch::Prefix,
                "",
                RuleAction::Allow,
                RuleRisk::Inherit,
            ),
        ],
        ..Default::default()
    };
    let engine = engine_with(profile);
    assert!(!engine.permits(&[SandboxFact::new("shell_command", "sudo rm -rf /")]));
    assert!(engine.permits(&[SandboxFact::new("shell_command", "git status")]));
    // deny 命中时风险档不参与覆盖（闸门已直接拒绝）。
    assert_eq!(
        engine.risk_of(&[SandboxFact::new("shell_command", "rm -rf x")]),
        None
    );
}

#[test]
fn all_four_match_modes() {
    let profile = SandboxProfile {
        rules: vec![
            rule(
                "shell_command",
                RuleMatch::Exact,
                "git status",
                RuleAction::Allow,
                RuleRisk::Low,
            ),
            rule(
                "vdfs_path",
                RuleMatch::Prefix,
                "/home/me/docs",
                RuleAction::Allow,
                RuleRisk::Low,
            ),
            rule(
                "vdfs_path",
                RuleMatch::Glob,
                "/tmp/**/*.rs",
                RuleAction::Deny,
                RuleRisk::High,
            ),
            // 黑名单式表达：最后一条通配兜底放行其余路径。
            rule(
                "vdfs_path",
                RuleMatch::Glob,
                "*",
                RuleAction::Allow,
                RuleRisk::Inherit,
            ),
            rule(
                "url",
                RuleMatch::Contains,
                "evil.test",
                RuleAction::Deny,
                RuleRisk::High,
            ),
        ],
        ..Default::default()
    };
    let engine = engine_with(profile);
    assert!(engine.permits(&[SandboxFact::new("shell_command", "git status")]));
    assert!(!engine.permits(&[SandboxFact::new("shell_command", "git status --short")]));
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "/home/me/docs/a.md")]));
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "/tmp/x/a.rs")]));
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "/tmp/x/a.txt")]));
    assert!(!engine.permits(&[SandboxFact::new("url", "https://evil.test/x")]));
}

#[test]
fn allow_rule_risk_overrides_tool_risk() {
    let profile = SandboxProfile {
        rules: vec![rule(
            "shell_command",
            RuleMatch::Prefix,
            "git ",
            RuleAction::Allow,
            RuleRisk::High,
        )],
        ..Default::default()
    };
    let engine = engine_with(profile);
    assert_eq!(
        engine.risk_of(&[SandboxFact::new("shell_command", "git push --force")]),
        Some("high".to_string())
    );
    // 没命中规则不指定风险档，工具保留自己的低风险判定。
    assert_eq!(
        engine.risk_of(&[SandboxFact::new("shell_command", "ls")]),
        None
    );
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
fn legacy_allowed_facts_migrate_to_exact_allow_rules() {
    let config: SandboxConfig = serde_json::from_value(serde_json::json!({
        "sandboxes": {
            "strict": {
                "name": "严格",
                "allowed_facts": { "shell_command": ["git status", "ls"] },
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
    assert!(engine.permits(&[SandboxFact::new("shell_command", "ls")]));
    assert!(!engine.permits(&[SandboxFact::new("shell_command", "rm -rf /")]));
    assert!(engine
        .intercept("dangerous_tool", &serde_json::json!({}))
        .is_some());
}

#[test]
fn rules_roundtrip_in_archive_config() {
    let cfg: SandboxProfileConfig = serde_json::from_value(serde_json::json!({
        "name": "规则档",
        "rules": [{
            "kind": "vdfs_path",
            "match": "glob",
            "value": "/home/me/**",
            "action": "allow",
            "risk": "low"
        }]
    }))
    .unwrap();
    assert_eq!(cfg.rules.len(), 1);
    assert_eq!(cfg.rules[0].mode, RuleMatch::Glob);
    let value = serde_json::to_value(&cfg).unwrap();
    assert_eq!(value["rules"][0]["match"], "glob");
}

#[test]
fn approval_params_flow_from_selected_archive() {
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
    let empty = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::default())),
        Some("missing".to_string()),
    );
    assert_eq!(empty.approval_params(), (None, None));
}
