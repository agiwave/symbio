//! `setting/config.rs` 的单元测试 —— 缺省、空白与非法取值的降级。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 刚装配完 = 什么都没设置 ⇒ 不注入任何东西（宿主不替用户编名字）
#[test]
fn default_settings_are_empty() {
    let c = SettingConfig::default();
    assert!(c.is_empty());
    assert_eq!(c.display_name(), "");
    assert_eq!(c.verbosity(), None);
    assert_eq!(c.agents_file_name(), "AGENTS.md", "指令文件名有默认值");
}

/// 空白视同未设置：表单里敲几个空格不该产出「名字是一段空格」的指令
#[test]
fn whitespace_counts_as_unset() {
    let c = SettingConfig {
        display_name: "  \t\n".to_string(),
        description: " ".to_string(),
        reply_language: "".to_string(),
        verbosity: "  ".to_string(),
        agents_file_name: "AGENTS.md".to_string(),
    };
    assert!(c.is_empty(), "全空白仍视同未设置");
}

/// 只要有一项被设置就不再是空的
#[test]
fn any_field_makes_it_non_empty() {
    let c = SettingConfig {
        reply_language: "中文".to_string(),
        ..SettingConfig::default()
    };
    assert!(!c.is_empty());
}

/// 手写的 `PLUGIN.yml` 写错取值 → 按「不约束」处理，而不是把错值念给模型听
#[test]
fn unknown_verbosity_is_ignored() {
    for raw in ["", "  ", "CONCISE", "verbose", "简洁"] {
        let c = SettingConfig {
            verbosity: raw.to_string(),
            ..SettingConfig::default()
        };
        assert_eq!(c.verbosity(), None, "{raw:?} 应被忽略");
    }
    let c = SettingConfig {
        verbosity: " concise ".to_string(),
        ..SettingConfig::default()
    };
    assert_eq!(c.verbosity(), Some("简洁"), "去空白后应认得出");
}

/// 存量 / 手写的 `PLUGIN.yml` 缺字段时按缺省补齐
#[test]
fn missing_fields_fall_back_to_defaults() {
    let c: SettingConfig = serde_json::from_str("{}").unwrap();
    assert!(c.is_empty());

    let c: SettingConfig = serde_json::from_str(r#"{"display_name":"评审官"}"#).unwrap();
    assert_eq!(c.display_name(), "评审官");
    assert_eq!(c.reply_language(), "");
    assert_eq!(c.agents_file_name(), "AGENTS.md", "缺文件名也回落默认");
}

/// 指令文件名非法（路径分隔符 / 父段 / 空白）→ 回落默认值（防逃逸）
#[test]
fn invalid_agents_file_name_falls_back() {
    for raw in [
        "",
        "  ",
        "../AGENTS.md",
        "sub/AGENTS.md",
        "sub\\AGENTS.md",
        "..",
        ".",
    ] {
        let c = SettingConfig {
            agents_file_name: raw.to_string(),
            ..SettingConfig::default()
        };
        assert_eq!(c.agents_file_name(), "AGENTS.md", "{raw:?} 应回落默认");
    }
    let c = SettingConfig {
        agents_file_name: " CUSTOM.md ".to_string(),
        ..SettingConfig::default()
    };
    assert_eq!(c.agents_file_name(), "CUSTOM.md", "合法名去空白后保留");
}

/// 表单定义覆盖全部五个字段（加字段忘了进表单 = 用户改不到）
#[test]
fn definition_covers_every_field() {
    let d = config_definition();
    let keys: Vec<String> = d
        .sections
        .iter()
        .flat_map(|s| s.fields.iter())
        .map(|f| f.key.clone())
        .collect();
    assert_eq!(
        keys,
        vec![
            "agents_file_name",
            "display_name",
            "description",
            "reply_language",
            "verbosity"
        ],
        "字段与配置结构必须一一对应"
    );
}
