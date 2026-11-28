//! `apps/cli/src/args.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
    // 默认不硬编码任何 provider id —— 交由后端按系统目录配置解析，
    // 避免指向一个无预设、无密钥来源的"幽灵默认值"，首次运行必然失败。
    assert!(a.provider.is_none());
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
fn serve_subcommand_parses_server_options() {
    let cwd = std::env::current_dir().unwrap();
    let Command::Serve(args) = parse([
        "serve".to_string(),
        "--bind".to_string(),
        "0.0.0.0".to_string(),
        "--port=9232".to_string(),
        "--token".to_string(),
        "secret".to_string(),
        "--homedir".to_string(),
        "runtime".to_string(),
        "--frontend-dist".to_string(),
        "web".to_string(),
        "--max-connections".to_string(),
        "12".to_string(),
        "--allowed-origin".to_string(),
        "https://console.example".to_string(),
        "--verbose".to_string(),
    ])
    .unwrap() else {
        panic!("expected Serve");
    };
    assert_eq!(args.homedir, cwd.join("runtime"));
    assert_eq!(args.config.bind, "0.0.0.0");
    assert_eq!(args.config.port, 9232);
    assert_eq!(args.config.token, "secret");
    assert_eq!(args.config.frontend_dist, Some(cwd.join("web")));
    assert_eq!(args.config.max_connections, 12);
    assert_eq!(
        args.config.allowed_origin.as_deref(),
        Some("https://console.example")
    );
    assert!(args.verbose);
}

#[test]
fn serve_help_and_invalid_values_are_explicit() {
    assert!(matches!(
        parse(["serve".to_string(), "--help".to_string()]),
        Ok(Command::ServeHelp)
    ));
    assert!(parse([
        "serve".to_string(),
        "--max-connections".to_string(),
        "0".to_string()
    ])
    .is_err());
    assert!(parse([
        "serve".to_string(),
        "--port".to_string(),
        "70000".to_string()
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
