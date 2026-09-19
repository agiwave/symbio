//! Telegram 插件的配置 schema（只被本插件消费，故定义在插件内部）

use serde::{Deserialize, Serialize};

/// Telegram configuration - Single Source of Truth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: Option<String>,
    #[serde(default = "default_true")]
    pub streaming_enabled: bool,
    #[serde(default = "default_true")]
    pub poll_enabled: bool,
    #[serde(default, deserialize_with = "user_ids")]
    pub allowed_users: Vec<i64>,
}

fn default_true() -> bool {
    true
}

/// `allowed_users` 的宽松反序列化：同时接受数字数组与数字字符串数组。
///
/// 落盘形态是数字数组（`PLUGIN.yml`），而配置文件的 `list` widget 提交的是
/// **每行一项的字符串数组**。两种形态都是「用户 ID 列表」，转换放在这里，
/// 插件不必为此分出一套中间类型。
fn user_ids<'de, D>(de: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    let raw = Vec::<serde_json::Value>::deserialize(de)?;
    raw.iter()
        .map(|v| match v {
            serde_json::Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| D::Error::custom(format!("用户 ID 必须是整数：{n}"))),
            serde_json::Value::String(s) => s
                .trim()
                .parse::<i64>()
                .map_err(|_| D::Error::custom(format!("「{s}」不是合法的 Telegram 用户 ID"))),
            other => Err(D::Error::custom(format!("用户 ID 必须是整数：{other}"))),
        })
        .collect()
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: "".to_string(),
            chat_id: None,
            streaming_enabled: true,
            poll_enabled: true,
            allowed_users: Vec::new(),
        }
    }
}

pub mod telegram_send {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Request {
        pub text: String,
        pub chat_id: Option<String>,
        pub parse_mode: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub sent: i32,
        pub message: String,
    }
}

pub mod telegram_status {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Response {
        pub configured: bool,
        pub has_chat_id: bool,
        pub streaming_enabled: bool,
        pub poll_enabled: bool,
        pub listener_running: bool,
        pub update_offset: i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 宽松反序列化必须同时吃下「数字数组」（落盘形态）与「字符串数组」
    /// （配置 widget 的 list 每行一项）——两种形态都表示同一份用户 ID 列表。
    #[test]
    fn allowed_users_accepts_numbers_and_strings() {
        let c: TelegramConfig =
            serde_json::from_str(r#"{"bot_token":"t","allowed_users":[1,2,3]}"#).unwrap();
        assert_eq!(c.allowed_users, vec![1, 2, 3]);

        let c: TelegramConfig =
            serde_json::from_str(r#"{"bot_token":"t","allowed_users":["10"," 20 ","30"]}"#)
                .unwrap();
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
        let back: TelegramConfig =
            serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.bot_token, "tok");
        assert_eq!(back.chat_id.as_deref(), Some("-100"));
        assert!(!back.streaming_enabled);
        assert_eq!(back.allowed_users, vec![7]);
    }
}
