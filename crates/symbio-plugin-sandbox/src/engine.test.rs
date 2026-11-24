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
        Arc::new(std::sync::RwLock::new(
            SandboxRegistry::try_from_profiles(Vec::new()).unwrap(),
        )),
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
    let decision = engine.decide(
        &[SandboxFact::new("shell_command", "rm -rf /")],
        "any_tool",
        &serde_json::json!({}),
    );
    assert!(decision.permitted);
    assert_eq!(decision.profile_version, 0);
    assert_eq!(decision.reason, None);
}

#[test]
fn unloaded_registry_denies_even_without_an_explicit_selection() {
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::default())),
        None,
    );
    assert!(!engine.permits(&[]));
    assert!(engine.intercept("tool", &Value::Null).is_some());
    let decision = engine.decide(&[], "tool", &Value::Null);
    assert!(!decision.permitted);
    assert!(decision
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("no successfully loaded snapshot")));
}

#[test]
fn no_selected_profile_uses_current_registry_version() {
    let mut registry = SandboxRegistry::default();
    registry.replace(
        SandboxRegistry::try_from_profiles(vec![(
            "present".into(),
            SandboxProfileConfig::default(),
        )])
        .unwrap(),
    );
    let engine = Engine::new(Arc::new(std::sync::RwLock::new(registry)), None);
    let decision = engine.decide(&[], "tool", &Value::Null);
    assert!(decision.permitted);
    assert_eq!(decision.profile_version, 1);
    assert_eq!(decision.reason, None);
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
fn path_rules_fail_closed_for_unresolved_shell_arguments() {
    let profile = SandboxProfile {
        rules: vec![rule(
            "vdfs_path",
            RuleMatch::Prefix,
            "/workspace",
            RuleAction::Allow,
            RuleRisk::Inherit,
        )],
        ..Default::default()
    };
    let engine = engine_with(profile);
    let decision = engine.decide(
        &[
            SandboxFact::new("shell_command", "cat README"),
            SandboxFact::new("vdfs_path", "/workspace"),
            SandboxFact::new("shell_unresolved_argument", "README"),
        ],
        "shell",
        &Value::Null,
    );
    assert!(!decision.permitted);
    assert!(decision
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("shell_unresolved_argument")));
}

#[test]
fn drive_relative_paths_cannot_hide_behind_absolute_rules() {
    let profile = SandboxProfile {
        rules: vec![
            rule(
                "vdfs_path",
                RuleMatch::Prefix,
                "/workspace/secret",
                RuleAction::Deny,
                RuleRisk::High,
            ),
            rule(
                "vdfs_path",
                RuleMatch::Glob,
                "*",
                RuleAction::Allow,
                RuleRisk::Inherit,
            ),
        ],
        ..Default::default()
    };
    let engine = engine_with(profile);
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "C:secret/key")]));
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "/workspace/secret/key")]));
}

#[test]
fn explicit_unresolved_argument_rules_can_allow_known_shell_words() {
    let profile = SandboxProfile {
        rules: vec![
            rule(
                "vdfs_path",
                RuleMatch::Prefix,
                "/workspace",
                RuleAction::Allow,
                RuleRisk::Inherit,
            ),
            rule(
                "shell_unresolved_argument",
                RuleMatch::Exact,
                "status",
                RuleAction::Allow,
                RuleRisk::Inherit,
            ),
        ],
        ..Default::default()
    };
    let engine = engine_with(profile);
    assert!(engine.permits(&[
        SandboxFact::new("vdfs_path", "/workspace"),
        SandboxFact::new("shell_unresolved_argument", "status"),
    ]));
    assert!(!engine.permits(&[
        SandboxFact::new("vdfs_path", "/workspace"),
        SandboxFact::new("shell_unresolved_argument", "README"),
    ]));
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
fn allow_rule_risk_is_a_floor_not_an_override() {
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
        Arc::new(std::sync::RwLock::new(
            SandboxRegistry::from_config(config).unwrap(),
        )),
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
    let reg = SandboxRegistry::try_from_profiles(profiles).unwrap();
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

#[test]
fn missing_selected_profile_denies_instead_of_allowing() {
    let valid = SandboxRegistry::try_from_profiles(vec![(
        "present".into(),
        SandboxProfileConfig::default(),
    )])
    .unwrap();
    let mut registry = SandboxRegistry::default();
    registry.replace(valid);
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("missing".into()),
    );

    assert!(!engine.permits(&[]));
    assert!(engine.intercept("tool", &serde_json::json!({})).is_some());
    let decision = engine.decide(&[], "tool", &serde_json::json!({}));
    assert!(!decision.permitted);
    assert_eq!(decision.profile_version, 1);
    assert!(decision
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("missing") && reason.contains("missing or invalid")));
}

#[test]
fn strict_threshold_is_low() {
    let registry = SandboxRegistry::try_from_profiles(vec![(
        "strict".into(),
        SandboxProfileConfig {
            approval_mode: Some("interactive".into()),
            risk_threshold: Some("low".into()),
            ..Default::default()
        },
    )])
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("strict".into()),
    );
    let decision = engine.decide(&[], "tool", &serde_json::json!({}));
    assert_eq!(decision.risk_threshold.as_deref(), Some("low"));
    assert_eq!(decision.approval_mode.as_deref(), Some("interactive"));
}

#[test]
fn decision_uses_one_versioned_registry_snapshot() {
    let shared = Arc::new(std::sync::RwLock::new(SandboxRegistry::default()));
    let first = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            approval_mode: Some("interactive".into()),
            risk_threshold: Some("low".into()),
            blocked_tools: std::collections::HashMap::from([(
                "dangerous".into(),
                serde_json::json!({"approval": "required"}),
            )]),
            ..Default::default()
        },
    )])
    .unwrap();
    shared.write().unwrap().replace(first);
    let engine = Engine::new(shared.clone(), Some("test".into()));
    let facts = [SandboxFact::new("shell_command", "git status")];
    let before = engine.decide(&facts, "dangerous", &serde_json::json!({}));
    assert_eq!(before.profile_version, 1);
    assert!(!before.permitted);
    assert_eq!(before.approval_mode.as_deref(), Some("interactive"));
    assert_eq!(before.risk_threshold.as_deref(), Some("low"));
    assert_eq!(
        before.intercept,
        Some(serde_json::json!({"approval": "required"}))
    );

    let second = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            approval_mode: Some("auto".into()),
            risk_threshold: Some("high".into()),
            ..Default::default()
        },
    )])
    .unwrap();
    shared.write().unwrap().replace(second);
    let after = engine.decide(&facts, "dangerous", &serde_json::json!({}));
    assert_eq!(after.profile_version, 2);
    assert!(after.permitted);
    assert_eq!(after.approval_mode.as_deref(), Some("auto"));
    assert_eq!(after.risk_threshold.as_deref(), Some("high"));
    assert_eq!(after.intercept, None);
}

#[test]
fn rule_risk_is_the_maximum_floor_across_facts() {
    let registry = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![
                rule(
                    "shell_command",
                    RuleMatch::Exact,
                    "low",
                    RuleAction::Allow,
                    RuleRisk::Low,
                ),
                rule(
                    "shell_command",
                    RuleMatch::Exact,
                    "high",
                    RuleAction::Allow,
                    RuleRisk::High,
                ),
            ],
            ..Default::default()
        },
    )])
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("test".into()),
    );
    for facts in [
        vec![
            SandboxFact::new("shell_command", "low"),
            SandboxFact::new("shell_command", "high"),
        ],
        vec![
            SandboxFact::new("shell_command", "high"),
            SandboxFact::new("shell_command", "low"),
        ],
    ] {
        assert_eq!(engine.risk_of(&facts).as_deref(), Some("high"));
        assert_eq!(
            engine.decide(&facts, "tool", &Value::Null).risk.as_deref(),
            Some("high")
        );
    }
}

#[test]
fn vdfs_path_prefix_matches_components_but_shell_prefix_stays_raw() {
    let registry = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![
                rule(
                    "vdfs_path",
                    RuleMatch::Prefix,
                    "/foo",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
                rule(
                    "shell_command",
                    RuleMatch::Prefix,
                    "/foo",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
            ],
            ..Default::default()
        },
    )])
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("test".into()),
    );
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "/foo")]));
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "/foo/bar")]));
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "/foobar")]));
    assert!(engine.permits(&[SandboxFact::new("shell_command", "/foobar")]));
}

#[test]
fn vdfs_path_rules_are_normalized_before_matching_and_duplicate_checks() {
    let registry = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![
                rule(
                    "vdfs_path",
                    RuleMatch::Prefix,
                    r"/foo//./bar",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
                rule(
                    "vdfs_path",
                    RuleMatch::Exact,
                    r"src\lib",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
            ],
            ..Default::default()
        },
    )])
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("test".into()),
    );
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "/foo/bar/baz")]));
    assert!(engine.permits(&[SandboxFact::new("vdfs_path", "src/lib")]));

    let duplicate = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![
                rule(
                    "vdfs_path",
                    RuleMatch::Exact,
                    r"src\lib",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
                rule(
                    "vdfs_path",
                    RuleMatch::Exact,
                    "src/lib",
                    RuleAction::Deny,
                    RuleRisk::Inherit,
                ),
            ],
            ..Default::default()
        },
    )]);
    assert!(duplicate.is_err());
}

#[test]
fn path_rules_cannot_be_shadowed_by_parent_segments() {
    let registry = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![
                rule(
                    "vdfs_path",
                    RuleMatch::Prefix,
                    "/safe",
                    RuleAction::Allow,
                    RuleRisk::Inherit,
                ),
                rule(
                    "vdfs_path",
                    RuleMatch::Prefix,
                    "/secret",
                    RuleAction::Deny,
                    RuleRisk::Inherit,
                ),
            ],
            ..Default::default()
        },
    )])
    .unwrap();
    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(registry)),
        Some("test".into()),
    );
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "/safe/../secret")]));
    assert!(!engine.permits(&[SandboxFact::new("vdfs_path", "../../secret")]));

    let invalid = SandboxRegistry::try_from_profiles(vec![(
        "test".into(),
        SandboxProfileConfig {
            rules: vec![rule(
                "vdfs_path",
                RuleMatch::Exact,
                "../secret",
                RuleAction::Allow,
                RuleRisk::Inherit,
            )],
            ..Default::default()
        },
    )]);
    let errors = invalid.unwrap_err();
    assert!(errors.iter().any(|error| {
        error.field == "rules.0.value" && error.message.contains("不能越出根目录")
    }));
}

#[test]
fn unchanged_reload_does_not_advance_profile_version() {
    let shared = Arc::new(std::sync::RwLock::new(SandboxRegistry::default()));
    let make = || {
        SandboxRegistry::try_from_profiles(vec![(
            "test".into(),
            SandboxProfileConfig {
                risk_threshold: Some("low".into()),
                ..Default::default()
            },
        )])
        .unwrap()
    };
    shared.write().unwrap().replace(make());
    shared.write().unwrap().replace(make());
    assert_eq!(shared.read().unwrap().version(), 1);
}

#[test]
fn invalid_rule_fields_and_duplicate_identities_are_rejected() {
    let config = SandboxProfileConfig {
        rules: vec![
            rule(" ", RuleMatch::Exact, "x", RuleAction::Allow, RuleRisk::Low),
            rule(
                "shell_command",
                RuleMatch::Exact,
                "  ",
                RuleAction::Allow,
                RuleRisk::Low,
            ),
            rule(
                "shell_command",
                RuleMatch::Exact,
                "git",
                RuleAction::Allow,
                RuleRisk::Low,
            ),
            rule(
                "shell_command",
                RuleMatch::Exact,
                "git",
                RuleAction::Deny,
                RuleRisk::High,
            ),
        ],
        ..Default::default()
    };
    let errors = SandboxRegistry::try_from_profiles(vec![("test".into(), config)]).unwrap_err();
    let fields: Vec<&str> = errors.iter().map(|error| error.field.as_str()).collect();
    assert!(fields.contains(&"rules.0.kind"));
    assert!(fields.contains(&"rules.1.value"));
    assert!(fields.contains(&"rules.3.value"));
}

#[test]
fn invalid_approval_values_and_profile_ids_are_rejected() {
    let bad_mode = SandboxProfileConfig {
        approval_mode: Some("manual".into()),
        ..Default::default()
    };
    let errors = SandboxRegistry::try_from_profiles(vec![("test".into(), bad_mode)]).unwrap_err();
    assert_eq!(errors[0].field, "approval_mode");

    let bad_threshold = SandboxProfileConfig {
        risk_threshold: Some("critical".into()),
        ..Default::default()
    };
    let errors =
        SandboxRegistry::try_from_profiles(vec![("test".into(), bad_threshold)]).unwrap_err();
    assert_eq!(errors[0].field, "risk_threshold");

    let errors = SandboxRegistry::try_from_profiles(vec![
        ("  ".into(), SandboxProfileConfig::default()),
        ("dup".into(), SandboxProfileConfig::default()),
        ("dup".into(), SandboxProfileConfig::default()),
    ])
    .unwrap_err();
    assert!(errors.iter().any(|error| error.field == "id"));
}

#[test]
fn empty_rules_are_valid_but_invalid_candidate_never_becomes_unrestricted() {
    assert!(SandboxRegistry::try_from_profiles(vec![(
        "empty".into(),
        SandboxProfileConfig::default(),
    )])
    .is_ok());
    assert!(SandboxRegistry::try_from_profiles(vec![(
        "bad".into(),
        SandboxProfileConfig {
            risk_threshold: Some("unlimited".into()),
            ..Default::default()
        },
    )])
    .is_err());

    let engine = Engine::new(
        Arc::new(std::sync::RwLock::new(SandboxRegistry::default())),
        Some("bad".into()),
    );
    assert!(!engine.decide(&[], "tool", &Value::Null).permitted);
}
