//! `symbio/src/plugins/telegram/schemas.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 宽松反序列化必须同时吃下「数字数组」（落盘形态）与「字符串数组」
/// （配置 widget 的 list 每行一项）——两种形态都表示同一份用户 ID 列表。
#[test]
fn allowed_users_accepts_numbers_and_strings() {
    let c: TelegramConfig =
        serde_json::from_str(r#"{"bot_token":"t","allowed_users":[1,2,3]}"#).unwrap();
    assert_eq!(c.allowed_users, vec![1, 2, 3]);

    let c: TelegramConfig =
        serde_json::from_str(r#"{"bot_token":"t","allowed_users":["10"," 20 ","30"]}"#).unwrap();
    assert_eq!(
        c.allowed_users,
        vec![10, 20, 30],
        "字符串项要按 trim 后的整数解析"
    );

    let c: TelegramConfig =
        serde_json::from_str(r#"{"bot_token":"t","allowed_users":[1,"2"]}"#).unwrap();
    assert_eq!(c.allowed_users, vec![1, 2], "同一列表允许混合形态");
}

#[test]
fn defaults_apply_when_fields_are_absent() {
    let c: TelegramConfig = serde_json::from_str(r#"{"bot_token":"t"}"#).unwrap();
    assert!(c.allowed_users.is_empty());
    assert!(c.streaming_enabled, "两个开关缺省为 true");
    assert!(c.poll_enabled);
    assert!(c.chat_id.is_none());
}

#[test]
fn allowed_users_rejects_non_numeric_entries() {
    for bad in [
        r#"{"bot_token":"t","allowed_users":["abc"]}"#,
        r#"{"bot_token":"t","allowed_users":[true]}"#,
        r#"{"bot_token":"t","allowed_users":[1.5]}"#,
    ] {
        assert!(
            serde_json::from_str::<TelegramConfig>(bad).is_err(),
            "应拒绝：{bad}"
        );
    }
}

#[test]
fn config_roundtrips_through_json() {
    let c = TelegramConfig {
        bot_token: "tok".into(),
        chat_id: Some("-100".into()),
        streaming_enabled: false,
        poll_enabled: true,
        allowed_users: vec![7],
    };
    let back: TelegramConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back.bot_token, "tok");
    assert_eq!(back.chat_id.as_deref(), Some("-100"));
    assert!(!back.streaming_enabled);
    assert_eq!(back.allowed_users, vec![7]);
}

/// 容器会为「必需插件」补建一份只含**身份键**的 `PLUGIN.yml`
/// （`plugin_provider` / `plugin_name`），业务键此时都还没写。
/// 结构级 `#[serde(default)]` 让这份「最正常的初始状态」解析成功，而不是被
/// 当成「配置损坏」打 WARN 再回落默认值（否则每次启动一条假告警）。
#[test]
fn identity_only_manifest_falls_back_to_defaults() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = crate::symbio_core::PluginDir::at(tmp.path().join("telegram"), "telegram");
    dir.ensure_manifest().unwrap();

    let cfg = dir
        .load::<TelegramConfig>()
        .expect("缺业务键不应报错（否则启动时打假告警）")
        .expect("目录已有身份键，应视为「存在配置」而非「没有配置」");

    assert_eq!(cfg.bot_token, "", "未配置的 token 为空串，即「未启用」");
    assert_eq!(cfg.chat_id, None);
    assert!(cfg.streaming_enabled && cfg.poll_enabled, "开关取默认 true");
    assert!(cfg.allowed_users.is_empty());
}

/// `#[serde(default)]` 只兜「缺键」，不掩「类型不符」——真损坏必须仍报错。
#[test]
fn wrong_type_still_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = crate::symbio_core::PluginDir::at(tmp.path().join("telegram"), "telegram");
    dir.ensure_manifest().unwrap();
    std::fs::write(
        dir.config_path(),
        "plugin_provider: telegram\nplugin_name: telegram\nbot_token: {a: 1}\n",
    )
    .unwrap();

    assert!(
        dir.load::<TelegramConfig>().is_err(),
        "类型不符属真损坏，不得被默认值静默吞掉"
    );
}
