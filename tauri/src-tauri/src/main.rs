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

/// 本产物「由哪份源码构建」的指纹。
///
/// 值来自**构建戳**（`.symbio-tauri.build-stamp`，与可执行文件同目录），由
/// `scripts/tauri-binary.mjs --stamp` 在构建**前**写入（`tauri.conf.json` 的
/// `beforeDevCommand` / `beforeBuildCommand` 会先调它一次）。指纹本身是
/// 「构建输入的内容 sha256」，算法与诊断入口都在那个脚本里（唯一真相）。
///
/// ## 为什么在**运行时**读，而不是编译期嵌入
///
/// 编译期嵌入要么在 `build.rs` 里再实现一遍哈希（两份必须逐字节一致的实现，
/// 迟早分叉），要么依赖 `rerun-if-changed` 的语义——而**一旦 emit 任何一条
/// `rerun-if-changed`，cargo 的默认启发式（「包内任何文件变了就重跑」）就失效**，
/// 漏掉一个输入的表现恰好是「代码是新的、指纹是旧的」，正是这套机制要消灭的那种
/// 不一致。读一个文件则是零风险的。
///
/// ## 取不到时为什么是 `unknown`
///
/// 打包分发时不带戳文件，直接用 `cargo build` 构建时也不带。此时**宁可自报
/// 「来源不明」，也不编一个看起来像指纹的值**——后者会让「对不上」这个信号
/// 失去意义，而「对不上」正是这行日志存在的全部理由。
fn build_id() -> String {
    const STAMP: &str = ".symbio-tauri.build-stamp";
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(STAMP)))
        .and_then(|stamp| std::fs::read_to_string(stamp).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn main() {
    // 初始化 tracing 日志（开发用 pretty，生产用 JSON 由环境变量决定）
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(true))
        .init();

    // **首行即「我是谁」**：任何一段日志因此自带「由哪份源码产出」，与
    // `node scripts/tauri-binary.mjs --print` 一比即知是否同源。
    //
    // 排查日志时最贵的一步就是确认这件事。少了这一行，日志里出现一条当前源码里
    // 不存在的格式时，第一反应会是去读代码找「是不是没接上」——而真相往往是跑的
    // 是过期产物，排查方向从第一步就错了（已发生过一次，见
    // `scripts/tauri-binary.mjs` 的文件头）。
    info!(
        build = %build_id(),
        version = env!("CARGO_PKG_VERSION"),
        "Symbio shell starting"
    );

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
            commands::route_v2,
            commands::route_v2_send,
            commands::route_v2_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
