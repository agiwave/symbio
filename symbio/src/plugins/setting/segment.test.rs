//! `setting/segment.rs` 的单元测试 —— 片段的**有则说、无则省**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 全空 ⇒ 不注入（缺省状态不改变任何一轮的提示词）
#[test]
fn empty_settings_produce_no_segment() {
    assert_eq!(segment(&SettingConfig::default()), None);
}

/// 只有档案
#[test]
fn profile_only() {
    let c = SettingConfig {
        display_name: "评审官".to_string(),
        description: "只读代码、不改代码".to_string(),
        ..SettingConfig::default()
    };
    let s = segment(&c).expect("有内容必注入");
    assert!(s.starts_with("【本智能体】"), "{s}");
    assert!(s.contains("名称：评审官"), "{s}");
    assert!(s.contains("简介：只读代码、不改代码"), "{s}");
    assert!(!s.contains("回复语言"), "没填的项不该出现：{s}");
    assert!(!s.contains("回答详略"), "没填的项不该出现：{s}");
}

/// 只有偏好
#[test]
fn preference_only() {
    let c = SettingConfig {
        reply_language: "中文".to_string(),
        verbosity: "concise".to_string(),
        ..SettingConfig::default()
    };
    let s = segment(&c).unwrap();
    assert!(s.contains("回复语言：中文"), "{s}");
    assert!(s.contains("回答详略：简洁"), "{s}");
    assert!(!s.contains("名称"), "{s}");
}

/// 四项齐全：一项一行，用分号连成一句（每轮都进上下文，不铺开解释）
#[test]
fn all_fields_are_one_line() {
    let c = SettingConfig {
        display_name: "评审官".to_string(),
        description: "只读代码".to_string(),
        reply_language: "中文".to_string(),
        verbosity: "detailed".to_string(),
    };
    let s = segment(&c).unwrap();
    assert_eq!(
        s,
        "【本智能体】名称：评审官；简介：只读代码；回复语言：中文；回答详略：详细"
    );
}

/// 填了但被判为无效（详略写错）⇒ 那一项不出现，其余照旧
#[test]
fn invalid_verbosity_is_dropped_not_spoken() {
    let c = SettingConfig {
        display_name: "评审官".to_string(),
        verbosity: "verbose".to_string(),
        ..SettingConfig::default()
    };
    let s = segment(&c).unwrap();
    assert!(!s.contains("回答详略"), "认不出的取值不该念给模型听：{s}");
}
