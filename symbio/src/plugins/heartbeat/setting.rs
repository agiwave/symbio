//! `Session.metadata.heartbeat` 那块数据的形状
//!
//! ## 它为什么在本插件里，却写进别人的存储
//!
//! 会话 metadata 是**透传的场景数据**：存储侧（`plugins/session`）只按键做浅合并，
//! 不认识任何键的内容（`heartbeat` 与 `my_plugin_flag` 对它而言是同一种东西）。
//! 「这块数据是什么意思」的 owner 是本插件——读写它的两半（[`crate::plugins::heartbeat::scheduler`]
//! 的扫描与 [`crate::plugins::heartbeat::tool`] 的设置入口）因此共用这一个形状，
//! 而不是各拼一份 JSON。
//!
//! 唯一的例外要说清：会话写臂在推导自主轮的运行参数时会读 `heartbeat.include_history`
//! 这一格（[D-076](README.md "D-076") #5 的裁决——运行参数由服务端按
//! 会话自己的数据推导，不从写体随身带）。那是**读**，不是第二份定义。
//!
//! ## 落盘的通道
//!
//! 写走 `vdfs/write(session/<id>)` 的 `{"metadata": {…}}` 浅合并：**按顶层键整块替换**，
//! 与本结构的「一次写全部四格」正好同义（部分更新在 [`crate::plugins::heartbeat::tool`] 里
//! 由先读后写完成，不在存储侧）。

use serde::{Deserialize, Serialize};

/// 会话心跳任务配置
///
/// 存储于 `Session.metadata.heartbeat`，由会话设置页（[`super::option`]）或模型侧工具
/// （[`super::tool`]）写入；[`super::scheduler`] 据此判定「这个会话现在该不该开一轮」。
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

    /// 配置的对外视图（与 `metadata.heartbeat` 的存储结构一致）。
    ///
    /// 设置工具的每次回执都回这一份：配置型写入的历史版本对后续推理没有参考价值，
    /// 回显生效值才是那一格该留下的东西。
    pub fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "interval_seconds": self.interval_seconds,
            "prompt": self.prompt,
            "include_history": self.include_history,
        })
    }
}

#[cfg(test)]
#[path = "setting.test.rs"]
mod tests;
