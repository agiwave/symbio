// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Allow dead code during development
#![allow(dead_code)]

mod commands;
mod event_sender;

use symbio::{create_root_plugin, OptionalEventSender, Plugin, ConnectionManager};
use event_sender::TauriEventSender;
use tauri::Manager;
use std::sync::{Mutex, Arc};

/// 全局 AppHandle（用于插件发送事件）
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// 获取全局 AppHandle
pub fn get_app_handle() -> Option<tauri::AppHandle> {
    APP_HANDLE.get().cloned()
}

struct AppState {
    root: Mutex<Arc<dyn Plugin>>,
    connection_manager: Mutex<Arc<ConnectionManager>>,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 保存全局 AppHandle
            APP_HANDLE.set(app.handle().clone()).ok();
            eprintln!("[main] AppHandle saved globally");

            // 创建 Tauri 事件发送器
            let event_sender = TauriEventSender::new(app.handle().clone());

            // 创建连接管理器
            let connection_manager = Arc::new(ConnectionManager::new(
                Arc::new(TauriEventSender::new(app.handle().clone())),
                1800, // 30 分钟超时
            ));

            // 创建 root plugin（包含所有子插件注册）
            let root = create_root_plugin(OptionalEventSender::new(Some(Arc::new(event_sender))));
            eprintln!("[main] Root plugin created with all plugins registered");

            app.manage(AppState {
                root: Mutex::new(root),
                connection_manager: Mutex::new(connection_manager),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::meta,
            commands::invoke,
            commands::stream,
            commands::connect,
            commands::connect_send,
            commands::connect_close,
            commands::connect_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}