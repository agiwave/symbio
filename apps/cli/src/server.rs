use std::process::ExitCode;
use std::sync::Arc;

use symbio_runtime::{EmbeddedUi, RuntimeHost};

use crate::args::ServeArgs;

/// CLI 入口把编译期内嵌的 `packages/ui/dist` 注入共享 runtime。
///
/// 内嵌逻辑就住在 [`crate::embedded_ui`]（CLI 自己的模块，不再单独成 crate）：
/// 只有需要 serve 的 CLI 二进制才把它编进自身；桌面启动器不内嵌 UI，
/// 嵌自己的 UI，共享 runtime 不依赖任何资源 crate，因此不会被迫嵌入第二份 dist。
struct CliEmbeddedUi;

impl EmbeddedUi for CliEmbeddedUi {
    fn get(path: &str) -> Option<&'static [u8]> {
        crate::embedded_ui::get(path)
    }

    fn content_type(path: &str) -> &'static str {
        crate::embedded_ui::content_type(path)
    }
}

pub async fn run(args: ServeArgs) -> ExitCode {
    let log_level = if args.verbose {
        symbio_platform::sdk::logger::LOG_LEVEL_DEBUG
    } else {
        symbio_platform::sdk::logger::LOG_LEVEL_INFO
    };
    symbio_platform::sdk::logger::logger_set_min_level(log_level);

    if let Err(error) = std::fs::create_dir_all(&args.homedir) {
        eprintln!("创建系统目录失败 {}: {error}", args.homedir.display());
        return ExitCode::FAILURE;
    }
    std::env::set_var("SYMBIO_HOMEDIR", &args.homedir);
    if !super::has_model_entry(&args.homedir) {
        eprintln!(
            "提示: 系统目录 {} 下没有模型配置，浏览器中的模型调用可能不可用。",
            args.homedir.display()
        );
    }

    let host = match RuntimeHost::create(symbio::init::root_factory()).await {
        Ok(host) => host,
        Err(error) => {
            eprintln!("运行时启动失败: {error}");
            return ExitCode::FAILURE;
        }
    };
    // 占位页是「没跑前端构建」的信号：明确告警，绝不静默当成正常产物。
    if crate::embedded_ui::is_placeholder() && args.config.frontend_dist.is_none() {
        eprintln!("警告: CLI 内嵌的是占位页（packages/ui/dist 未构建）；serve 的 UI 将不可用。");
        eprintln!("      在 packages/ui 下运行 npm run build 后重新构建 CLI。");
    }

    let needs_token = !args.config.token.is_empty();
    let config = args.config;
    let handle = match symbio_runtime::start::<CliEmbeddedUi>(config, Arc::clone(&host)).await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("Server 启动失败: {error}");
            if let Err(error) = host.shutdown().await {
                eprintln!("RuntimeHost 停止失败: {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    let url = format!("http://{}", handle.local_addr());
    println!("Symbio Server 已启动: {url}");

    if args.browser {
        match open_browser(&url, needs_token) {
            Ok(()) => println!("已在系统默认浏览器中打开 {url}"),
            Err(error) => eprintln!("无法打开浏览器（服务仍在运行）: {error}\n请手动访问 {url}"),
        }
    }

    if wait_for_shutdown_signal().await.is_err() {
        eprintln!("无法监听退出信号，Server 将继续运行");
    }
    symbio_runtime::stop(handle).await;
    if let Err(error) = host.shutdown().await {
        eprintln!("RuntimeHost 停止失败: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// 用系统默认浏览器打开本机服务地址。
///
/// 只走标准库 `Command`，不引入 `open` / `webbrowser` 依赖（最小依赖树约定）。
/// 带 token 的 Server 不把令牌拼进 URL——那是凭据泄漏；此时返回错误让调用方
/// 打印地址由用户手动打开。桌面启动器走的是回环免 token 路径，不受此限制。
fn open_browser(url: &str, needs_token: bool) -> Result<(), String> {
    if needs_token {
        return Err("Server 启用了访问令牌，请在浏览器手动打开并配置 Bearer Token".to_string());
    }
    // 环境变量可能携带用户偏好；空值一律回落到平台默认命令。
    let (program, args): (&str, Vec<String>) = if cfg!(windows) {
        (
            "cmd",
            vec!["/C".into(), "start".into(), "".into(), url.into()],
        )
    } else if cfg!(target_os = "macos") {
        ("open", vec![url.into()])
    } else {
        ("xdg-open", vec![url.into()])
    };
    std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动 {program} 失败: {e}"))
}

#[cfg(windows)]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c()?;
    let mut ctrl_break = tokio::signal::windows::ctrl_break()?;
    tokio::select! {
        _ = ctrl_c.recv() => {}
        _ = ctrl_break.recv() => {}
    }
    Ok(())
}

#[cfg(not(windows))]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    tokio::signal::ctrl_c().await
}
