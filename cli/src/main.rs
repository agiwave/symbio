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
use client::{vdfs_change_of, SymbioClient};
use render::Renderer;
use symbio::symbio_core::vdfs_provider::{
    VDFS_OUTCOME_ABORTED, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING,
};

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

    // 系统目录下没有任何模型条目时给出明确提示：此时插件树会退回内置默认值，
    // 通常表现为「没有任何可用 Provider」，属于最常见的一次性配置问题。
    //
    // 注意这里看的是**模型条目目录**而不是某个配置文件：配置已回到插件目录
    // （系统目录下 `model/<id>/provider.json`），旧的集中式 `config.yaml`
    // 只会在首次迁移后被改名留档，不能再拿它当判据。
    if !has_model_entry(&args.homedir) {
        eprintln!(
            "提示: 系统目录 {} 下没有模型配置（model/<id>/provider.json），\
             将使用内置默认配置（可能没有可用模型）。",
            args.homedir.display()
        );
        eprintln!("      在应用左侧导航的「模型」里添加一个模型，或直接放一份 provider.json。");
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

    // 判定使用方式：心跳守护 > 显式消息 > 强制 REPL > 管道输入（非终端）> 交互 REPL
    if args.heartbeat {
        return run_heartbeat_daemon(client, &args).await;
    }
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

/// 心跳守护模式：常驻宿主 session 插件的后台心跳调度器。
///
/// 心跳机制完全在后端 session 插件内闭环：配置存 `Session.metadata.heartbeat`，
/// 调度循环随插件树构建启动（15s 扫描各会话的空闲心跳任务）。本进程只需保持
/// 存活，调度器就会按各会话的空闲节奏自动触发心跳对话；消费无人值守轮次的
/// 活动即可，Ctrl+C 退出。
///
/// ## 订阅面比普通模式宽一级
///
/// 心跳可能落在**任何一个**已登记心跳的会话上，而守护进程在启动期不知道将来
/// 会有哪些——所以闸门开在**会话挂载根**（`<根>/session`）上，而不是某个会话。
/// 判据仍然是会话节点的运行态：`status` 离开 `working` 即为本轮结束，
/// 结局从 `attributes` 读（`error` = 失败，`outcome == aborted` = 中止）。
async fn run_heartbeat_daemon(mut client: SymbioClient, args: &args::Args) -> ExitCode {
    if !args.quiet {
        eprintln!("Symbio CLI（心跳守护模式）");
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
        eprintln!("  心跳调度器随插件树常驻运行；Ctrl+C 退出。");
        eprintln!();
    }

    let mount = match client.watch_all_sessions().await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✖ 订阅会话变更失败: {e}");
            return ExitCode::FAILURE;
        }
    };
    // 会话级地址 = 挂载根 + **一段**；再深的都是消息节点（不参与运行态汇报）
    let mount_prefix = format!("{mount}/");
    // 各会话上一次见到的运行态：只在**迁移**上报。会话叶子的变更也包含标题 /
    // 元数据写入，逐帧报会把一次心跳刷成十几行。
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    while let Some(ev) = client.next_bus_event().await {
        let Some(change) = vdfs_change_of(&ev) else {
            continue;
        };
        let Some(node) = change.node.as_ref() else {
            continue;
        };
        let Some(sid) = change.path.strip_prefix(mount_prefix.as_str()) else {
            continue;
        };
        if sid.is_empty() || sid.contains('/') {
            continue;
        }
        if seen.get(sid) == Some(&node.status) {
            continue;
        }
        seen.insert(sid.to_string(), node.status.clone());
        if args.quiet {
            continue;
        }

        match node.status.as_str() {
            VDFS_STATUS_WORKING => eprintln!("▶ [{sid}] 心跳触发，开始工作"),
            VDFS_STATUS_FAILED => {
                let err = node
                    .attributes
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("本轮以错误结束");
                eprintln!("✖ [{sid}] {err}");
            }
            _ if node.attributes.get("outcome").and_then(|v| v.as_str())
                == Some(VDFS_OUTCOME_ABORTED) =>
            {
                eprintln!("■ [{sid}] 本轮被中止");
            }
            _ => eprintln!("■ [{sid}] 本轮收敛，回到空闲"),
        }
    }

    if !args.quiet {
        eprintln!("事件总线已关闭，守护退出。");
    }
    ExitCode::SUCCESS
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

/// 系统目录下是否至少有一个模型条目（系统目录下 `model/<id>/provider.json`）。
///
/// 只做**目录形态**判据：条目内容是否可用由后端插件在构造时自己判定（读不出来的
/// 条目会各自跳过），CLI 不重复那套校验——这里只回答「有没有配置过模型」。
fn has_model_entry(homedir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(homedir.join("model")) else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.path().is_dir() && e.path().join("provider.json").is_file())
}
