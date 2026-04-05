//! 事件发送器抽象
//!
//! 提供平台无关的事件发送机制，允许插件向外部发送事件
//! 具体实现由宿主平台（如 Tauri）注入

use serde::Serialize;
use std::sync::Arc;

/// 事件发送器 trait
///
/// 插件通过此接口向宿主平台发送事件，实现平台无关性
/// 
/// 注意：为了保持 object-safe，使用 serde_json::Value 作为数据格式
pub trait EventSender: Send + Sync {
    /// 发送事件
    ///
    /// # 参数
    /// - `event_name`: 事件名称
    /// - `payload`: 事件数据（已序列化为 JSON）
    fn emit(&self, event_name: &str, payload: serde_json::Value) -> Result<(), String>;
}

/// 可选的事件发送器包装器（允许插件在没有事件发送器时工作）
#[derive(Clone)]
pub struct OptionalEventSender {
    sender: Option<Arc<dyn EventSender>>,
}

impl OptionalEventSender {
    pub fn new(sender: Option<Arc<dyn EventSender>>) -> Self {
        Self { sender }
    }

    pub fn emit<T: Serialize + Send>(&self, event_name: &str, payload: T) -> Result<(), String> {
        if let Some(sender) = &self.sender {
            let json = serde_json::to_value(&payload)
                .map_err(|e| format!("Failed to serialize payload: {}", e))?;
            sender.emit(event_name, json)
        } else {
            // 没有事件发送器时静默忽略
            Ok(())
        }
    }

    pub fn is_some(&self) -> bool {
        self.sender.is_some()
    }
}
