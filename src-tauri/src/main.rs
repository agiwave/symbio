// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod plugins;
mod commands;

use core::{PluginFactoryRegistry, Plugin, PluginFactory};
use plugins::{
    HomeFactory, WorkFactory, SettingFactory,
    AgentFactory, ChatFactory, ToolsFactory, MemoryFactory, 
    SessionFactory, TelegramFactory, OpenAiFactory,
    EchoFactory, DockerFactory, CompositeFactory,
};
use std::sync::{Mutex, Arc};

struct AppState {
    root: Mutex<Arc<dyn Plugin>>,
}

fn main() {
    // 初始化全局工厂注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();

    // 注册所有工厂
    registry.register(Arc::new(WorkFactory::new()));
    registry.register(Arc::new(SettingFactory::new()));
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