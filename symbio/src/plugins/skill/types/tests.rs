//! Skill 类型定义 单元测试
//!
//! 对应源文件: `types.rs`

use super::*;
use serde_json::json;

/// TEST-S2.1：SkillConfig::default() 4 个字段默认值
#[test]
fn skill_config_default_values() {
    let cfg = SkillConfig::default();
    assert_eq!(cfg.skill_dirs, vec![".symbio/skills".to_string()]);
    assert_eq!(cfg.max_skills, 20);
    assert_eq!(cfg.max_body_chars, 8000);
    assert!(cfg.report_token_estimate);
}

/// TEST-S2.2：SkillConfig JSON 反序列化使用默认值
#[test]
fn skill_config_deserialize_uses_defaults_for_missing_fields() {
    let cfg: SkillConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(cfg.skill_dirs, vec![".symbio/skills".to_string()]);
    assert_eq!(cfg.max_skills, 20);
    assert_eq!(cfg.max_body_chars, 8000);
    assert!(cfg.report_token_estimate);
}

/// TEST-S2.3：SkillConfig 反序列化支持覆盖所有字段
#[test]
fn skill_config_deserialize_overrides_all() {
    let cfg: SkillConfig = serde_json::from_str(
        r#"{
            "skill_dirs": ["a", "b"],
            "max_skills": 5,
            "max_body_chars": 1000,
            "report_token_estimate": false
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.skill_dirs, vec!["a", "b"]);
    assert_eq!(cfg.max_skills, 5);
    assert_eq!(cfg.max_body_chars, 1000);
    assert!(!cfg.report_token_estimate);
}

/// TEST-S1.1：单个变量替换
#[test]
fn substitute_variables_single() {
    let skill = Skill {
        name: "x".into(),
        description: "x".into(),
        body: "Hello, ${name}!".into(),
        allowed_tools: None,
        argument_hint: None,
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "x.md".into(),
    };
    let out = skill.substitute_variables(&skill.body, &json!({ "name": "world" }));
    assert_eq!(out, "Hello, world!");
}

/// TEST-S1.2：多变量替换
#[test]
fn substitute_variables_multi() {
    let skill = Skill {
        name: "x".into(),
        description: "x".into(),
        body: "${greeting}, ${name}! You are ${age}.".into(),
        allowed_tools: None,
        argument_hint: None,
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "x.md".into(),
    };
    let out = skill.substitute_variables(
        &skill.body,
        &json!({ "greeting": "Hi", "name": "Alice", "age": 30 }),
    );
    // JSON 字符串值不再序列化为带引号，数字仍然是裸值 30
    assert_eq!(out, "Hi, Alice! You are 30.");
}

/// TEST-S1.3：变量不替换占位符时原样保留
#[test]
fn substitute_variables_missing_key_kept() {
    let skill = Skill {
        name: "x".into(),
        description: "x".into(),
        body: "Hello, ${name}!".into(),
        allowed_tools: None,
        argument_hint: None,
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "x.md".into(),
    };
    let out = skill.substitute_variables(&skill.body, &json!({}));
    assert_eq!(out, "Hello, ${name}!");
}

/// TEST-S1.4：variables 不是 object 时不替换
#[test]
fn substitute_variables_non_object_noop() {
    let skill = Skill {
        name: "x".into(),
        description: "x".into(),
        body: "Hello, ${name}!".into(),
        allowed_tools: None,
        argument_hint: None,
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "x.md".into(),
    };
    let out = skill.substitute_variables(&skill.body, &json!([1, 2, 3]));
    assert_eq!(out, "Hello, ${name}!");
}

/// TEST-S1.5：同一变量在文本中多次出现全部替换
#[test]
fn substitute_variables_repeating_key() {
    let skill = Skill {
        name: "x".into(),
        description: "x".into(),
        body: "${x}-${x}-${x}".into(),
        allowed_tools: None,
        argument_hint: None,
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "x.md".into(),
    };
    let out = skill.substitute_variables(&skill.body, &json!({ "x": "Y" }));
    assert_eq!(out, "Y-Y-Y");
}
