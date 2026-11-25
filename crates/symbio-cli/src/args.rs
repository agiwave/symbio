//! 命令行参数解析。
//!
//! 刻意手写而不引入 clap：本项目坚持最小依赖树（新增依赖要走一次联网预热，
//! 与「cargo 全程离线」的约定冲突），而这里需要的只是一个十来行的状态机。

use std::path::PathBuf;

/// CLI 默认会话运行模式。
///
/// `auto`（无人值守）：CLI 没有交互卡的应答入口，遇到需审批的工具时
/// 应让后端返回友好错误继续，而不是挂起等待一个永远不会到来的点击。
/// 人在环的审批流属于 Tauri 前端的职责；CLI 只播报到达的等待卡片。
pub const DEFAULT_MODE: &str = crate::protocol::MODE_AUTO;

/// 解析结果。
pub enum Command {
    Run(Box<Args>),
    Serve(Box<ServeArgs>),
    Help,
    ServeHelp,
    Version,
}

#[derive(Debug, Clone)]
pub struct ServeArgs {
    pub homedir: PathBuf,
    pub config: symbio_runtime::ServerConfig,
    pub verbose: bool,
}

#[derive(Debug, Clone)]
pub struct Args {
    /// 一次性消息（`-m` 或位置参数）。非空即进入非交互模式。
    pub message: Option<String>,
    /// 复用 / 继续的会话 id；缺省时新建一个会话。
    pub session: Option<String>,
    /// Model Provider id；`None` 表示交由后端按系统目录配置解析。
    pub provider: Option<String>,
    /// 会话运行模式：auto / interactive。
    pub mode: String,
    /// 会话工作目录（工具调用的上下文根）。
    pub workdir: PathBuf,
    /// 系统目录（homedir）：插件配置与会话存储的根。
    pub homedir: PathBuf,
    /// 可选绑定的 Agent id。
    pub agent: Option<String>,
    /// 静默：非交互模式下不打印进度/工具行。
    pub quiet: bool,
    /// 详细：额外打印模型推理（Reasoning）内容。
    pub verbose: bool,
    /// 强制进入交互式 REPL，即使标准输入不是终端。
    ///
    /// 默认行为是「stdin 不是终端 ⇒ 走管道非交互模式」，这对 `echo x | cli`
    /// 是正确的，但会挡住两类场景：① 终端不支持 TTY 时仍想用 REPL；
    /// ② 用脚本喂多轮输入来自动化验证会话。给一个显式开关即可两全。
    pub repl: bool,
    /// 心跳守护模式：常驻宿主插件树里的心跳调度插件（可选装配）。
    ///
    /// 心跳机制完全在后端闭环（配置存 `Session.metadata.heartbeat`，调度循环随插件树
    /// 构建启动，判空闲与开轮各住心跳调度插件与会话插件（实现锚点见各插件自述文档）。CLI 只需
    /// 保持进程存活，调度器就会按各会话的空闲节奏自动触发心跳对话；本进程同时渲染
    /// 事件总线上的会话活动。
    pub heartbeat: bool,
}

const HELP: &str = "\
symbio-cli — Symbio 命令行前端（纯 Rust）

用法:
  symbio-cli [选项] [消息...]        一次性发送消息后退出（非交互）
  symbio-cli [选项]                  进入交互式 REPL
  echo \"你好\" | symbio-cli [选项]    从标准输入读取消息（非交互）
  symbio-cli serve [选项]           启动浏览器可访问的 Server

选项:
  -m, --message <文本>    发送单条消息后退出（等价于位置参数）
  -s, --session <ID>      复用既有会话（默认新建）
      --provider <ID>     Model Provider（默认不指定，用系统目录配置的模型；
                          显式传 ID 才覆盖，default 是回退到系统配置的兼容写法）
      --mode <MODE>       会话运行模式 auto|interactive（默认 auto）
      --workdir <路径>    会话工作目录（默认当前目录）
      --homedir <路径>    系统目录（默认 <当前目录>/.symbio）
      --agent <ID>        绑定 Agent（可选）
  -i, --repl              强制进入交互式 REPL（即使 stdin 不是终端）
      --heartbeat         心跳守护模式：常驻宿主后台心跳调度器（会话空闲达到
                          设定间隔后自动触发对话），Ctrl+C 退出
  -q, --quiet             非交互模式下只输出模型文本
  -v, --verbose           打印模型推理内容与更多诊断
  -h, --help              显示本帮助
  -V, --version           显示版本

Server 模式:
  symbio-cli serve --help 查看 Server 专属选项

交互模式内置命令:
  /help                   显示命令列表
  /new                    新建会话
  /session <ID>           切换/续接会话
  /provider <ID>          切换 Provider（default = 系统默认）
  /workdir <路径>         切换工作目录
  /stats                  查看当前会话读数
  /exit, /quit            退出

输出约定:
  标准输出 = 模型文本 + 交互提示符（可直接管道给下游程序）
  标准错误 = 进度、工具调用、错误、日志（可 2>/dev/null 静音）
  等待用户动作（审批卡 / 提问卡）会在标准错误明确播报：CLI 没有应答入口。
  auto 下本轮以未完成收尾；interactive 下停在等用户，请在桌面端处理该卡片。
";

const SERVE_HELP: &str = "\
symbio-cli serve — 启动浏览器可访问的 Symbio Server

用法:
  symbio-cli serve [选项]

Server 选项:
      --bind <地址>          监听地址（默认 127.0.0.1）
      --port <端口>          监听端口（默认 9231）
      --token <令牌>         Bearer Token；非回环地址必填
      --homedir <路径>       系统目录（默认 <当前目录>/.symbio）
      --frontend-dist <路径> Vue 构建产物（默认 tauri/dist）
      --max-connections <数>  并发连接上限（默认 64）
      --allowed-origin <源>   允许的精确浏览器 Origin；设置时必须同时提供 token
  -v, --verbose              显示更多诊断
  -h, --help                 显示本帮助
  -V, --version              显示版本

环境变量:
  SYMBIO_SERVER_BIND / SYMBIO_SERVER_PORT / SYMBIO_SERVER_TOKEN
  SYMBIO_SERVER_ALLOWED_ORIGIN / SYMBIO_FRONTEND_DIST / SYMBIO_SERVER_MAX_CONNECTIONS
";

pub fn help_text() -> &'static str {
    HELP
}

pub fn serve_help_text() -> &'static str {
    SERVE_HELP
}

/// 解析进程参数。
pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Command, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("无法获取当前目录: {e}"))?;
    let mut it = argv.into_iter().peekable();
    if it.peek().is_some_and(|arg| arg == "serve") {
        it.next();
        return parse_serve(it, &cwd);
    }

    let mut args = Args {
        message: None,
        session: None,
        provider: None,
        mode: DEFAULT_MODE.to_string(),
        workdir: cwd.clone(),
        homedir: cwd.join(".symbio"),
        agent: None,
        quiet: false,
        verbose: false,
        repl: false,
        heartbeat: false,
    };

    let mut positional: Vec<String> = Vec::new();

    while let Some(raw) = it.next() {
        // 支持 `--key=value` 与 `--key value` 两种写法
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with('-') => (f.to_string(), Some(v.to_string())),
            _ => (raw.clone(), None),
        };

        // 取值的统一入口：优先用 `=` 内联值，否则吃掉下一个参数
        let mut take_value = |name: &str, inline: Option<String>| -> Result<String, String> {
            if let Some(v) = inline {
                return Ok(v);
            }
            it.next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("{name} 缺少取值"))
        };

        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-m" | "--message" => {
                let v = take_value("--message", inline)?;
                args.message = Some(match args.message.take() {
                    Some(prev) => format!("{prev} {v}"),
                    None => v,
                });
            }
            "-s" | "--session" => args.session = Some(take_value("--session", inline)?),
            "--provider" => {
                let v = take_value("--provider", inline)?;
                args.provider = if v.eq_ignore_ascii_case("default") {
                    None
                } else {
                    Some(v)
                };
            }
            "--mode" => args.mode = take_value("--mode", inline)?,
            "--workdir" => args.workdir = resolve_path(&take_value("--workdir", inline)?, &cwd),
            "--homedir" => args.homedir = resolve_path(&take_value("--homedir", inline)?, &cwd),
            "--agent" => args.agent = Some(take_value("--agent", inline)?),
            "-q" | "--quiet" => args.quiet = true,
            "-v" | "--verbose" => args.verbose = true,
            "-i" | "--repl" => args.repl = true,
            "--heartbeat" => args.heartbeat = true,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("未知选项: {other}（用 --help 查看用法）"));
            }
            _ => positional.push(raw),
        }
    }

    if !positional.is_empty() {
        let joined = positional.join(" ");
        args.message = Some(match args.message.take() {
            Some(prev) => format!("{prev} {joined}"),
            None => joined,
        });
    }

    // 守护模式与一次性消息语义互斥：守护进程只宿主调度器，不代发消息。
    if args.heartbeat && args.message.is_some() {
        return Err(
            "--heartbeat 守护模式不能与消息同用（守护进程只宿主调度器，不发送消息）".to_string(),
        );
    }

    Ok(Command::Run(Box::new(args)))
}

fn parse_serve<I: IntoIterator<Item = String>>(
    argv: I,
    cwd: &std::path::Path,
) -> Result<Command, String> {
    let mut config = symbio_runtime::ServerConfig::default();
    config.bind = std::env::var("SYMBIO_SERVER_BIND").unwrap_or(config.bind);
    config.token = std::env::var("SYMBIO_SERVER_TOKEN").unwrap_or(config.token);
    config.allowed_origin = std::env::var("SYMBIO_SERVER_ALLOWED_ORIGIN").ok();
    config.port = std::env::var("SYMBIO_SERVER_PORT")
        .ok()
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| "SYMBIO_SERVER_PORT 必须是 0–65535".to_string())
        })
        .transpose()?
        .unwrap_or(config.port);
    config.max_connections = std::env::var("SYMBIO_SERVER_MAX_CONNECTIONS")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "SYMBIO_SERVER_MAX_CONNECTIONS 必须是正整数".to_string())
        })
        .transpose()?
        .unwrap_or(config.max_connections);
    config.frontend_dist = std::env::var("SYMBIO_FRONTEND_DIST")
        .map(|value| resolve_path(&value, cwd))
        .unwrap_or(config.frontend_dist);

    let mut args = ServeArgs {
        homedir: cwd.join(".symbio"),
        config,
        verbose: false,
    };
    let mut it = argv.into_iter();
    while let Some(raw) = it.next() {
        let (flag, inline) = match raw.split_once('=') {
            Some((flag, value)) if flag.starts_with('-') => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (raw.clone(), None),
        };
        let mut take_value = |name: &str, inline: Option<String>| -> Result<String, String> {
            if let Some(value) = inline {
                return Ok(value);
            }
            it.next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{name} 缺少取值"))
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::ServeHelp),
            "-V" | "--version" => return Ok(Command::Version),
            "--bind" => args.config.bind = take_value("--bind", inline)?,
            "--port" => {
                args.config.port = take_value("--port", inline)?
                    .parse()
                    .map_err(|_| "--port 必须是 0–65535".to_string())?
            }
            "--token" => args.config.token = take_value("--token", inline)?,
            "--allowed-origin" => {
                args.config.allowed_origin = Some(take_value("--allowed-origin", inline)?)
            }
            "--homedir" => args.homedir = resolve_path(&take_value("--homedir", inline)?, cwd),
            "--frontend-dist" => {
                args.config.frontend_dist =
                    resolve_path(&take_value("--frontend-dist", inline)?, cwd)
            }
            "--max-connections" => {
                args.config.max_connections = take_value("--max-connections", inline)?
                    .parse()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "--max-connections 必须是正整数".to_string())?
            }
            "-v" | "--verbose" => args.verbose = true,
            other if other.starts_with('-') => {
                return Err(format!(
                    "未知 Server 选项: {other}（用 serve --help 查看用法）"
                ))
            }
            _ => return Err(format!("serve 不接受位置参数: {raw}")),
        }
    }
    Ok(Command::Serve(Box::new(args)))
}

/// 相对路径按调用时的当前目录解析，`~` 前缀交给核心库展开。
fn resolve_path(raw: &str, cwd: &std::path::Path) -> PathBuf {
    let p = std::path::Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    cwd.join(p)
}

#[cfg(test)]
#[path = "args.test.rs"]
mod tests;
