// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod plugins;
mod commands;

use core::{PluginFactoryRegistry, Plugin};
use plugins::{AgentFactory, EchoFactory, CalculatorFactory, FormatterFactory};
use std::sync::{Mutex, Arc};

struct AppState {
    root: Mutex<Arc<dyn Plugin>>,
}

fn main() {
    // 初始化全局工厂注册表
    PluginFactoryRegistry::init();
    let registry = PluginFactoryRegistry::global();
    
    // 插件主动注册自己的工厂
    registry.register(Arc::new(EchoFactory::new()));
    registry.register(Arc::new(CalculatorFactory::new()));
    registry.register(Arc::new(FormatterFactory::new()));
    registry.register(Arc::new(AgentFactory::new()));
    
    // 使用 AgentFactory 创建 root agent
    let root: Arc<dyn Plugin> = registry
        .list()
        .into_iter()
        .find(|f| f.meta().name == "agent")
        .expect("AgentFactory should be registered")
        .create(None, None);
    
    tauri::Builder::default()
        .manage(AppState {
            root: Mutex::new(root),
        })
        .invoke_handler(tauri::generate_handler![
            commands::meta,
            commands::invoke,
            commands::sinvoke
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
