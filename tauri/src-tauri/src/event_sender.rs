//! Tauri 事件发送器实现
//!
//! 将 Tauri 的 AppHandle 包装为 symbio 的 EventSender trait

use symbio::EventSender;
use tauri::AppHandle;

/// Tauri 事件发送器
///
/// 实现 symbio 的 EventSender trait，将事件通过 Tauri 发送到前端
pub struct TauriEventSender {
    app_handle: AppHandle,
}

impl TauriEventSender {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

impl EventSender for TauriEventSender {
    fn emit(&self, event_name: &str, payload: serde_json::Value) -> Result<(), String> {
        use tauri::Emitter;
        self.app_handle
            .emit(event_name, &payload)
            .map_err(|e| format!("Failed to emit Tauri event: {}", e))
    }
}
