//! Symbio 桌面交付物：Tauri 原生窗口壳。
//!
//! 职责边界（刻意收窄）：
//! - 开一个原生窗口，加载本机 `symbio-cli serve` 提供的 UI；
//! - 启动时拉起 CLI 子进程（不带 `--browser`，窗口本身就是 UI）；
//! - 窗口关闭时终止 CLI 子进程。
//!
//! 不嵌 UI（UI 在 CLI 二进制里编译期内嵌）、不注册 IPC command（`route_v2` 家族已删除）、
//! 不依赖插件树 crate。前端与后端之间只有 HTTP/WS，桌面端与 cli serve、web 完全同构。
//!
//! `tauri dev` 时窗口加载 Vite dev server（cfg(dev)），前端 HMR 实时生效，
//! 其 `/api` 代理到本机 CLI serve——等价旧 `tauri dev` 的体验。
//! 直接运行二进制 / release 则加载 CLI 内嵌 UI，不依赖 Vite。

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{Manager, WindowEvent};

/// CLI serve 子进程句柄；窗口关闭时终止它，避免留下孤儿后端进程。
struct CliProcess(Mutex<Option<Child>>);

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let port = server_port();
            let cli = resolve_cli_binary()?;
            let homedir = resolve_homedir()?;

            eprintln!(
                "Symbio 桌面：CLI={} homedir={}",
                cli.display(),
                homedir.display()
            );

            let child = Command::new(&cli)
                .arg("serve")
                .arg("--port")
                .arg(port.to_string())
                .arg("--homedir")
                .arg(&homedir)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| format!("无法启动 CLI（{}）: {e}", cli.display()))?;

            app.manage(CliProcess(Mutex::new(Some(child))));

            wait_for_server(port, Duration::from_secs(30))?;

            // `tauri dev` 时 Tauri CLI 会设 `cfg(dev)`，此时窗口加载 Vite dev server
            // （HMR 实时生效，其 /api 代理到本机 CLI serve）——等价旧 `tauri dev` 体验。
            // 直接运行二进制 / release 则加载 CLI 内嵌 UI，不依赖 Vite。
            // SYMBIO_DESKTOP_URL 显式设置时优先级最高。
            let url = env::var("SYMBIO_DESKTOP_URL").unwrap_or_else(|_| {
                if cfg!(dev) {
                    "http://localhost:1420".to_string()
                } else {
                    format!("http://127.0.0.1:{port}")
                }
            });

            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "未找到 main 窗口".to_string())?;
            window
                .navigate(url.parse().map_err(|e| format!("非法地址 {url}: {e}"))?)
                .map_err(|e| format!("窗口导航到 {url} 失败: {e}"))?;
            window.show().map_err(|e| format!("显示窗口失败: {e}"))?;
            Ok(())
        })
        .on_window_event(|app, event| {
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(state) = app.try_state::<CliProcess>() {
                    if let Ok(mut guard) = state.0.lock() {
                        if let Some(child) = guard.as_mut() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        *guard = None;
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用运行失败");
}

/// CLI serve 端口：与 CLI 默认值一致，可用环境变量覆盖。
fn server_port() -> u16 {
    env::var("SYMBIO_SERVER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(9231)
}

/// 系统目录：显式 `SYMBIO_HOMEDIR` 优先，否则取平台用户配置目录下的 `symbio`。
///
/// 桌面端是「安装即用的应用」，不应把配置写进用户当时恰好所在的目录，
/// 而应落在稳定的用户配置根（与 CLI 直接运行的 cwd 默认不同）。
fn resolve_homedir() -> Result<PathBuf, String> {
    if let Some(raw) = env::var_os("SYMBIO_HOMEDIR") {
        let p = PathBuf::from(raw);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "无法确定用户配置目录，请设置 SYMBIO_HOMEDIR".to_string())?;
    Ok(base.join("symbio"))
}

/// 定位 CLI 二进制：`SYMBIO_CLI` 显式指定，否则按平台名找同目录兄弟文件，最后回落 PATH。
fn resolve_cli_binary() -> Result<PathBuf, String> {
    if let Some(raw) = env::var_os("SYMBIO_CLI") {
        let p = PathBuf::from(raw);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("SYMBIO_CLI 指向的文件不存在: {}", p.display()));
    }

    let exe = env::current_exe().map_err(|e| format!("无法定位当前可执行文件: {e}"))?;
    let sibling = exe
        .parent()
        .map(|dir| dir.join(cli_exe_name()))
        .ok_or_else(|| "无法定位可执行文件所在目录".to_string())?;
    if sibling.is_file() {
        return Ok(sibling);
    }
    let found = which_in_path(cli_exe_name()).ok_or_else(|| {
        format!(
            "找不到 CLI 二进制（同目录与 PATH 均未命中）: {}",
            sibling.display()
        )
    })?;
    Ok(found)
}

fn cli_exe_name() -> &'static str {
    if cfg!(windows) {
        "symbio-cli.exe"
    } else {
        "symbio-cli"
    }
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 轮询 health 端点直到就绪或超时。用裸 TCP 发一个最小 GET，避免为新壳引入 HTTP 客户端依赖。
fn wait_for_server(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();
    while Instant::now() < deadline {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                let req =
                    b"GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
                if stream.write_all(req).is_ok() {
                    let mut buf = [0u8; 64];
                    if let Ok(n) = stream.read(&mut buf) {
                        let line = String::from_utf8_lossy(&buf[..n]).to_string();
                        if line.contains(" 200 ") {
                            return Ok(());
                        }
                        last = line;
                    }
                }
            }
            Err(e) => last = e.to_string(),
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "CLI serve 未在 {timeout:?} 内就绪（最后响应: {last}）"
    ))
}

#[cfg(test)]
#[path = "main.test.rs"]
mod tests;
