//! Symbio CLI 前端入口。
//!
//! 与 `tauri/` 并列的第二种前端形态：**同一套插件树、同一份协议**，只是把
//! 传输层从 Tauri IPC 换成进程内直连 + 终端渲染。支持两种使用方式：
//!
//! - 非交互：`symbio-cli -m "问题"` 或 `echo 问题 | symbio-cli`（可管道、可脚本化）
//! - 交互：`symbio-cli`（REPL，多轮对话）

mod args;
mod client;
mod render;

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use tokio::io::{AsyncBufReadExt, BufReader};

use args::Command;
use client::SymbioClient;
use render::Renderer;

/// CLI 只做「解析 → 启动 → 发送 → 渲染」，业务全在后端插件树里。
///
/// 注意：刻意 **不** 调用 `symbio::init::initialize()`。
/// 那个函数会安装 tracing subscriber 并默认写 **stdout**（且默认过滤级别是
/// `info,symbio=debug`），会把大量插件日志混进模型正文。不初始化时，
/// 插件日志宏退回 `eprintln!` —— 正好落在 stderr，stdout 保持纯净。
#[tokio::main]
async fn main() -> ExitCode {
    let argv = std::env::args().skip(1);
    let cmd = match args::parse(argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("参数错误: {e}");
            return ExitCode::from(2);
        }
    };

    let args = match cmd {
        Command::Help => {
            print!("{}", args::help_text());
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("symbio-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Run(a) => *a,
    };

    // 系统目录缺失 config.yaml 时给出明确提示：此时插件树会退回内置默认值，
    // 通常表现为「没有任何可用 Provider」，属于最常见的一次性配置问题。
    if !args.homedir.join("config.yaml").exists() {
        eprintln!(
            "提示: 系统目录 {} 下没有 config.yaml，将使用内置默认配置（可能没有可用模型）。",
            args.homedir.display()
        );
        eprintln!("      如需指定其它系统目录，用 --homedir <路径>。");
    }

    let client = match SymbioClient::start(
        &args.homedir,
        &args.workdir,
        args.session.clone(),
        args.provider.clone(),
        args.mode.clone(),
        args.agent.clone(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("启动失败: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 判定使用方式：显式消息 > 强制 REPL > 管道输入（非终端）> 交互 REPL
    if args.message.is_some() {
        return run_once(client, args.message.clone(), &args).await;
    }
    if args.repl {
        return run_repl(client, &args).await;
    }
    if !std::io::stdin().is_terminal() {
        return run_once(client, None, &args).await;
    }
    run_repl(client, &args).await
}

/// 非交互：发送一条消息，成功则退出码 0。
async fn run_once(mut client: SymbioClient, inline: Option<String>, args: &args::Args) -> ExitCode {
    let mut renderer = Renderer::new(args.quiet, args.verbose);

    let message = match inline {
        Some(m) if !m.trim().is_empty() => m,
        _ => match read_stdin_all().await {
            Ok(s) if !s.trim().is_empty() => s,
            Ok(_) => {
                eprintln!("标准输入为空，没有可发送的消息。");
                return ExitCode::from(2);
            }
            Err(e) => {
                eprintln!("读取标准输入失败: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    renderer.notice(&format!(
        "会话 {} · Provider {} · 工作目录 {}",
        client.session_id,
        client.provider_label(),
        client.workdir
    ));

    match client.ask(message.trim(), &mut renderer).await {
        Ok(()) => {
            if !renderer.wrote_text {
                eprintln!("（模型没有产出文本内容，请检查 Provider 配置或用 --verbose 查看详情）");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✖ {e}");
            ExitCode::FAILURE
        }
    }
}

/// 交互式 REPL：多轮对话，内置少量会话级命令。
async fn run_repl(mut client: SymbioClient, args: &args::Args) -> ExitCode {
    let mut renderer = Renderer::new(args.quiet, args.verbose);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    if !args.quiet {
        eprintln!("Symbio CLI（交互模式）");
        eprintln!(
            "  系统目录 {} · 工作目录 {}",
            args.homedir.display(),
            client.workdir
        );
        eprintln!(
            "  会话 {} · Provider {}",
            client.session_id,
            client.provider_label()
        );
        eprintln!("  输入 /help 查看命令，/exit 退出（Ctrl+D 亦可）");
        eprintln!();
    }

    loop {
        print!("\x1b[36m你 ›\x1b[0m ");
        let _ = std::io::stdout().flush();

        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break, // Ctrl+D / EOF
            Err(e) => {
                eprintln!("读取输入失败: {e}");
                return ExitCode::FAILURE;
            }
        };

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if let Some(rest) = input.strip_prefix('/') {
            match handle_command(&mut client, rest).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => eprintln!("✖ {e}"),
            }
            continue;
        }

        if let Err(e) = client.ask(input, &mut renderer).await {
            eprintln!("✖ {e}");
        }
    }

    if !args.quiet {
        eprintln!("再见。");
    }
    ExitCode::SUCCESS
}

/// 处理 REPL 内置命令。返回 `Ok(false)` 表示退出。
async fn handle_command(client: &mut SymbioClient, cmd: &str) -> Result<bool, String> {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim();
    let arg = parts.next().unwrap_or("").trim();

    match name {
        "exit" | "quit" | "q" => Ok(false),
        "help" | "h" | "?" => {
            eprintln!(
                "命令:\n  /new                新建会话\n  /session <ID>       切换/续接会话\n  /provider [ID]      查看或切换 Provider（ID 传 default = 系统默认）\n  /workdir [路径]     查看或切换工作目录\n  /exit | /quit       退出"
            );
            Ok(true)
        }
        "new" => {
            let sid = format!("cli{}", chrono_like_stamp());
            client.switch_session(sid).await?;
            eprintln!("→ 新会话 {}", client.session_id);
            Ok(true)
        }
        "session" | "s" => {
            if arg.is_empty() {
                eprintln!("当前会话 {}", client.session_id);
                return Ok(true);
            }
            client.switch_session(arg.to_string()).await?;
            eprintln!("→ 已切换会话 {}", client.session_id);
            Ok(true)
        }
        "provider" | "p" => {
            if arg.is_empty() {
                eprintln!("当前 Provider {}", client.provider_label());
                return Ok(true);
            }
            client.switch_provider(if arg.eq_ignore_ascii_case("default") {
                None
            } else {
                Some(arg.to_string())
            });
            eprintln!("→ 已切换 Provider {}", client.provider_label());
            Ok(true)
        }
        "workdir" | "w" => {
            if arg.is_empty() {
                eprintln!("当前工作目录 {}", client.workdir);
                return Ok(true);
            }
            let path = std::path::Path::new(arg);
            let abs = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|e| format!("无法获取当前目录: {e}"))?
                    .join(path)
            };
            client.workdir = abs.to_string_lossy().to_string();
            // 工作目录属于会话元数据，改完立即落库（下一个请求才会带上新值）
            client.ensure_session().await?;
            eprintln!("→ 工作目录 {}", client.workdir);
            Ok(true)
        }
        other => {
            eprintln!("未知命令 /{other}（/help 查看列表）");
            Ok(true)
        }
    }
}

/// 简易时间戳（`/new` 用），避免为这一个用途引入时间库。
fn chrono_like_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:x}")
}

/// 读取全部标准输入（管道模式）。
async fn read_stdin_all() -> Result<String, String> {
    let mut buf = String::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(l) = lines.next_line().await.map_err(|e| e.to_string())? {
        buf.push_str(&l);
        buf.push('\n');
    }
    Ok(buf)
}
