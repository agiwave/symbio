// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Allow dead code during development
#![allow(dead_code)]

mod core;
mod plugins;
mod commands;

use core::{PluginFactoryRegistry, Plugin, PluginFactory};
use plugins::{
    HomeFactory, WorkFactory, NoteFactory, SettingFactory,
    AgentFactory, ChatFactory, ToolsFactory, MemoryFactory,
    SessionFactory, TelegramFactory, OpenAiFactory,
    EchoFactory, DockerFactory, CompositeFactory,
    ExplorerFactory,
};
use std::sync::{Mutex, Arc};

/// 全局 AppHandle（用于插件发送事件）
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// 获取全局 AppHandle
pub fn get_app_handle() -> Option<tauri::AppHandle> {
    APP_HANDLE.get().cloned()
}

struct AppState {
    root: Mutex<Arc<dyn Plugin>>,
}

fn main() {
    // 初始化全局工厂注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();

    // 注册所有工厂
    registry.register(Arc::new(WorkFactory::new()));
    registry.register(Arc::new(NoteFactory::new()));
    registry.register(Arc::new(SettingFactory::new()));
    registry.register(Arc::new(ExplorerFactory::new()));
    registry.register(Arc::new(AgentFactory::new()));
    registry.register(Arc::new(ChatFactory::new()));
    registry.register(Arc::new(ToolsFactory::new()));
    registry.register(Arc::new(MemoryFactory::new()));
    registry.register(Arc::new(SessionFactory::new()));
    registry.register(Arc::new(TelegramFactory::new()));
    registry.register(Arc::new(OpenAiFactory::new()));
    registry.register(Arc::new(EchoFactory::new()));
    registry.register(Arc::new(DockerFactory::new()));
    registry.register(Arc::new(CompositeFactory::with_defaults()));
    registry.register(Arc::new(HomeFactory::new()));

    // 创建 root 插件（HomeFactory 会自动读取配置文件并传给各子工厂）
    let root: Arc<dyn Plugin> = HomeFactory::new().create(None, None);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 保存全局 AppHandle
            APP_HANDLE.set(app.handle().clone()).ok();
            eprintln!("[main] AppHandle saved globally");
            Ok(())
        })
        .manage(AppState {
            root: Mutex::new(root),
        })
        .invoke_handler(tauri::generate_handler![
            commands::meta,
            commands::invoke,
            commands::stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}