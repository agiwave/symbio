//! Telegram 插件的配置 schema（只被本插件消费，故定义在插件内部）

use serde::{Deserialize, Serialize};

/// Telegram configuration - Single Source of Truth
///
/// `#[serde(default)]`（结构级）：**配置文件存在但未声明全部键**时按字段默认值补齐，
/// 而不是反序列化失败。这是常态而非异常——新建的 `PLUGIN.yml` 只有身份键，
/// `bot_token` 为空即「未启用」（`api_url()` 据此返回 `None`）。缺了这层默认，
/// 每次启动都会打一条「读取自身配置失败，改用默认值」的 WARN，把正常状态报成故障。
/// 只有 YAML 语法错误 / 类型不符这类**真正**的损坏才会走 `Err` 分支。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: Option<String>,
    /// Bot API 上游基址；缺省即官方端点（对现有部署零行为变化）。
    /// 指向自建 / 代理端点是网络受限环境的常见需求，也是 e2e 能打本地靶子的前提。
    pub api_base: String,
    #[serde(default = "default_true")]
    pub poll_enabled: bool,
    #[serde(default, deserialize_with = "user_ids")]
    pub allowed_users: Vec<i64>,
    /// `getUpdates` 的游标：已确认到的 `update_id + 1`。
    ///
    /// 它是**跨重启的运行态事实**，owner 与其它配置相同（`PLUGIN.yml`）：放进
    /// 第二个文件就会有两个真相。表单里可见、可改（改小 = 主动重放历史更新）。
    #[serde(default)]
    pub update_offset: i64,
}

fn default_true() -> bool {
    true
}

/// 官方 Bot API 端点（`api_url()` 的缺省基址）。
pub const OFFICIAL_API_BASE: &str = "https://api.telegram.org";

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
            api_base: OFFICIAL_API_BASE.to_string(),
            poll_enabled: true,
            allowed_users: Vec::new(),
            update_offset: 0,
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
        pub poll_enabled: bool,
        pub listener_running: bool,
        pub update_offset: i64,
    }
}

#[cfg(test)]
#[path = "schemas.test.rs"]
mod tests;
