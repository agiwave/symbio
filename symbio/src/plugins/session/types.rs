//! Session 类型定义

use serde::{Deserialize, Serialize};

pub use crate::symbio_core::schemas::session::chat_message::ChatMessage;

/// 会话数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: i64,
    pub updated_at: i64,
    pub metadata: serde_json::Value,
}

impl Session {
    pub fn new(id: impl Into<String>) -> Self {
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        Self {
            id: id.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::json!({}),
        }
    }
}

/// 会话心跳任务配置
///
/// 存储于 `Session.metadata.heartbeat`，由前端"会话设置"写入。
/// 后端 [`crate::plugins::session::SessionPlugin`] 的后台调度器据此在会话空闲
/// 指定时间后自动发起一次提示词对话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// 是否启用心跳任务
    #[serde(default)]
    pub enabled: bool,
    /// 启动间隔（秒）：会话空闲达到该时长后触发一次心跳
    #[serde(default = "default_heartbeat_interval")]
    pub interval_seconds: u64,
    /// 心跳任务提示词（每次触发时作为一条用户消息发送给模型）
    #[serde(default)]
    pub prompt: String,
    /// 启动心跳时是否携带历史会话信息
    /// - `true`（默认）：心跳消息作为普通对话追加，模型能看到历史
    /// - `false`：本次发送不加载任何历史（"无上下文"心跳）
    #[serde(default = "default_heartbeat_include_history")]
    pub include_history: bool,
}

fn default_heartbeat_interval() -> u64 {
    300
}

fn default_heartbeat_include_history() -> bool {
    true
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: default_heartbeat_interval(),
            prompt: String::new(),
            include_history: default_heartbeat_include_history(),
        }
    }
}

impl HeartbeatConfig {
    /// 从会话 metadata 解析心跳配置。字段缺失时返回默认（未启用）配置。
    pub fn from_metadata(metadata: &serde_json::Value) -> Self {
        let Some(obj) = metadata.get("heartbeat").and_then(|v| v.as_object()) else {
            return Self::default();
        };
        let enabled = obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let interval_seconds = obj
            .get("interval_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(default_heartbeat_interval);
        let prompt = obj
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let include_history = obj
            .get("include_history")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(default_heartbeat_include_history);
        Self {
            enabled,
            interval_seconds,
            prompt,
            include_history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_metadata_missing_returns_disabled_default() {
        let m = serde_json::json!({ "title": "x" });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(!hb.enabled);
        assert_eq!(hb.interval_seconds, 300);
        assert!(hb.include_history);
        assert!(hb.prompt.is_empty());
    }

    #[test]
    fn from_metadata_parses_full_config() {
        let m = serde_json::json!({
            "heartbeat": {
                "enabled": true,
                "interval_seconds": 120,
                "prompt": "请检查待办",
                "include_history": false
            }
        });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(hb.enabled);
        assert_eq!(hb.interval_seconds, 120);
        assert_eq!(hb.prompt, "请检查待办");
        assert!(!hb.include_history);
    }

    #[test]
    fn from_metadata_missing_fields_use_defaults() {
        let m = serde_json::json!({ "heartbeat": { "enabled": true } });
        let hb = HeartbeatConfig::from_metadata(&m);
        assert!(hb.enabled);
        assert_eq!(hb.interval_seconds, 300);
        assert!(hb.include_history);
    }
}
