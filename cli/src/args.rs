//! 命令行参数解析。
//!
//! 刻意手写而不引入 clap：本项目坚持最小依赖树（新增依赖要走一次联网预热，
//! 与「cargo 全程离线」的约定冲突），而这里需要的只是一个十来行的状态机。

use std::path::PathBuf;

/// CLI 默认使用的 Model Provider。
///
/// 之所以在这里给一个具体默认值（而不是交给后端 model 插件目录下的 `PLUGIN.yml` 的
/// `default_provider_id`）：CLI 的定位是「开箱即用地连上一个可用模型」，
/// 而 `--provider default` 显式保留「回退到系统目录配置」的能力。
pub const DEFAULT_PROVIDER: &str = "usrouter-glm5-3-flash";

/// CLI 默认会话运行模式。
///
/// `auto`（无人值守）：CLI 没有交互卡片的渲染能力，遇到需审批的工具时
/// 应让后端返回友好错误继续，而不是挂起等待一个永远不会到来的点击。
/// 人在环的审批流属于 Tauri 前端的职责。
pub const DEFAULT_MODE: &str = "auto";

/// 解析结果。
pub enum Command {
    Run(Box<Args>),
    Help,
    Version,
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
    /// 心跳守护模式：常驻宿主 session 插件的后台心跳调度器。
    ///
    /// 心跳机制完全在后端 session 插件内闭环（配置存 `Session.metadata.heartbeat`，
    /// 调度循环随插件树构建启动）。CLI 只需保持进程存活，调度器就会按各会话的
    /// 空闲节奏自动触发心跳对话；本进程同时渲染事件总线上的会话活动。
    pub heartbeat: bool,
}

const HELP: &str = "\
symbio-cli — Symbio 命令行前端（纯 Rust）

用法:
  symbio-cli [选项] [消息...]        一次性发送消息后退出（非交互）
  symbio-cli [选项]                  进入交互式 REPL
  echo \"你好\" | symbio-cli [选项]    从标准输入读取消息（非交互）

选项:
  -m, --message <文本>    发送单条消息后退出（等价于位置参数）
  -s, --session <ID>      复用既有会话（默认新建）
      --provider <ID>     Model Provider（默认 usrouter-glm5-3-flash；
                          传 default 表示回退到系统目录配置的默认 Provider）
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

交互模式内置命令:
  /help                   显示命令列表
  /new                    新建会话
  /session <ID>           切换/续接会话
  /provider <ID>          切换 Provider（default = 系统默认）
  /workdir <路径>         切换工作目录
  /exit, /quit            退出

输出约定:
  标准输出 = 模型文本 + 交互提示符（可直接管道给下游程序）
  标准错误 = 进度、工具调用、错误、日志（可 2>/dev/null 静音）
";

pub fn help_text() -> &'static str {
    HELP
}

/// 解析进程参数。
pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Command, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("无法获取当前目录: {e}"))?;

    let mut args = Args {
        message: None,
        session: None,
        provider: Some(DEFAULT_PROVIDER.to_string()),
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
    let mut it = argv.into_iter().peekable();

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

/// 相对路径按调用时的当前目录解析，`~` 前缀交给核心库展开。
fn resolve_path(raw: &str, cwd: &std::path::Path) -> PathBuf {
    let p = std::path::Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    cwd.join(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(argv: &[&str]) -> Args {
        match parse(argv.iter().map(|s| s.to_string())).unwrap() {
            Command::Run(a) => *a,
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn defaults_use_cwd_and_dot_symbio() {
        let a = run(&[]);
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(a.workdir, cwd);
        assert_eq!(a.homedir, cwd.join(".symbio"));
        assert_eq!(a.provider.as_deref(), Some(DEFAULT_PROVIDER));
        assert_eq!(a.mode, "auto");
        assert!(a.message.is_none());
    }

    #[test]
    fn inline_and_separated_values_both_work() {
        let a = run(&["--provider=x", "--session", "s1", "-m", "hi"]);
        assert_eq!(a.provider.as_deref(), Some("x"));
        assert_eq!(a.session.as_deref(), Some("s1"));
        assert_eq!(a.message.as_deref(), Some("hi"));
    }

    #[test]
    fn provider_default_sentinel_falls_back_to_none() {
        assert!(run(&["--provider", "default"]).provider.is_none());
        assert!(run(&["--provider", "DEFAULT"]).provider.is_none());
    }

    #[test]
    fn positional_args_join_into_message() {
        assert_eq!(run(&["你好", "世界"]).message.as_deref(), Some("你好 世界"));
    }

    #[test]
    fn unknown_flag_is_rejected() {
        assert!(parse(["--nope".to_string()]).is_err());
    }

    #[test]
    fn repl_flag_forces_interactive() {
        assert!(!run(&[]).repl);
        assert!(run(&["--repl"]).repl);
        assert!(run(&["-i"]).repl);
    }

    #[test]
    fn heartbeat_flag_is_bool_and_excludes_message() {
        assert!(!run(&[]).heartbeat);
        assert!(run(&["--heartbeat"]).heartbeat);
        assert!(parse([
            "--heartbeat".to_string(),
            "-m".to_string(),
            "hi".to_string()
        ])
        .is_err());
    }

    #[test]
    fn relative_paths_resolve_against_cwd() {
        // 绝对路径样例需带盘符才算 Windows 绝对路径（is_absolute 语义）。
        let abs = if cfg!(windows) {
            r"C:\abs\hd"
        } else {
            "/abs/hd"
        };
        let a = run(&["--workdir", "sub", "--homedir", abs]);
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(a.workdir, cwd.join("sub"));
        assert_eq!(a.homedir, PathBuf::from(abs));
    }
}
