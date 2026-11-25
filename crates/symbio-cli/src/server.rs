use std::process::ExitCode;
use std::sync::Arc;

use symbio_server::RuntimeHost;

use crate::args::ServeArgs;

pub async fn run(args: ServeArgs) -> ExitCode {
    let log_level = if args.verbose {
        symbio::symbio_sdk::logger::LOG_LEVEL_DEBUG
    } else {
        symbio::symbio_sdk::logger::LOG_LEVEL_INFO
    };
    symbio::symbio_sdk::logger::logger_set_min_level(log_level);

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

    let host = match RuntimeHost::create().await {
        Ok(host) => host,
        Err(error) => {
            eprintln!("运行时启动失败: {error}");
            return ExitCode::FAILURE;
        }
    };
    let config = args.config;
    let handle = match symbio_server::start(config, Arc::clone(&host)).await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("Server 启动失败: {error}");
            if let Err(error) = host.shutdown().await {
                eprintln!("RuntimeHost 停止失败: {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    println!("Symbio Server 已启动: http://{}", handle.local_addr());

    if wait_for_shutdown_signal().await.is_err() {
        eprintln!("无法监听退出信号，Server 将继续运行");
    }
    symbio_server::stop(handle).await;
    if let Err(error) = host.shutdown().await {
        eprintln!("RuntimeHost 停止失败: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
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
