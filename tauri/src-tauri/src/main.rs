mod commands;
mod route_connection;

use symbio::init::create_root_plugin;
use symbio::symbio_core::Plugin;
use route_connection::RouteConnectionManager;
use tauri::{Manager, Listener};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

struct AppState {
    root: Arc<dyn Plugin>,
    route_manager: Arc<RouteConnectionManager>,
}

fn main() {
    // 初始化 tracing 日志（开发用 pretty，生产用 JSON 由环境变量决定）
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(true))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            info!("Setup starting");

            // 创建 root plugin（包含所有子插件注册）
            let root = tauri::async_runtime::block_on(async {
                create_root_plugin().await
            });
            info!("Root plugin created with all plugins registered");

            let route_manager = Arc::new(RouteConnectionManager::new());

            // 在 Tokio runtime 中启动清理任务
            {
                let rm = route_manager.clone();
                tauri::async_runtime::block_on(async {
                    rm.start_cleanup_task();
                });
            }

            app.manage(AppState {
                root,
                route_manager: route_manager.clone(),
            });

            // 监听所有窗口事件
            app.listen_any("tauri://destroyed", move |_event| {
                info!("Window destroyed or reloaded, removing all active connections (without cancelling)");
                let rm = route_manager.clone();
                tauri::async_runtime::spawn(async move {
                    rm.remove_all().await;
                });
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // commands::meta,
            commands::route_v2,
            commands::route_v2_send,
            commands::route_v2_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
