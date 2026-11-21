//! `config.rs` 的单元测试 —— 默认值与前端快照同源、定义校验守住取值域。

use super::*;

#[test]
fn defaults_match_frontend_snapshot() {
    let c = UiPrefsConfig::default();
    assert_eq!(c.theme, "light");
    assert_eq!(c.font_size, "medium");
    assert!(c.sound_enabled);
    assert!(c.sound_completed && c.sound_aborted && c.sound_failed);
    assert_eq!(c.volume, DEFAULT_VOLUME);
    assert_eq!(c.volume, 0.6, "与前端 DEFAULT_CHIME_VOLUME 同源");
}

#[test]
fn definition_accepts_default_config() {
    let value = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    config_definition()
        .validate(&value)
        .expect("缺省配置必须能通过自身定义的校验");
}

#[test]
fn definition_rejects_out_of_domain_values() {
    let def = config_definition();

    let mut bad_theme = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    bad_theme["theme"] = serde_json::json!("neon");
    assert!(def.validate(&bad_theme).is_err(), "主题取值必须在候选内");

    let mut bad_volume = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    bad_volume["volume"] = serde_json::json!(1.5);
    assert!(def.validate(&bad_volume).is_err(), "音量不能超出 0..1");

    let mut bad_kind = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    bad_kind["sound_enabled"] = serde_json::json!("yes");
    assert!(def.validate(&bad_kind).is_err(), "开关必须是布尔值");
}

#[test]
fn deserialization_falls_back_to_defaults_for_missing_fields() {
    let c: UiPrefsConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.theme, "light");
    assert_eq!(c.volume, DEFAULT_VOLUME);
}
