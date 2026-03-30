// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod plugins;
mod commands;

use core::{PluginFactoryRegistry, Plugin, PluginFactory};
use plugins::{EchoFactory, CalculatorFactory, FormatterFactory, DockerFactory, HomeFactory};
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
    registry.register(Arc::new(DockerFactory::new()));
    
    // 使用 HomeFactory 创建 root 插件
    // Home 会自动创建 work/agent/setting 子插件实例
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