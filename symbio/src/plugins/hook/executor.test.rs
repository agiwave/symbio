//! `executor.rs` 的判据化测试。
//!
//! 聚焦**载荷怎么到钩子手里**——这条链路改过多次（workdir 临时文件 → heredoc →
//! 独立临时目录），每次换一种传参方式，而它对钩子作者是**契约**：命令的最后一个词
//! 是事件 JSON 的**路径**。测试就钉在这条契约上：路径真的以最后一个参数抵达、
//! 内容真的是那个事件。
//!
//! 附带钉住两条安全判据（都曾是真实缺陷）：
//! - 载荷内容**不进命令串**——事件里带 `;` / 重定向时不得被 shell 当命令执行；
//! - 临时文件不落在用户 workdir 里，且命令跑完即消失。

use super::*;
use crate::symbio_core::HookEvent;
use std::path::PathBuf;

/// 一条把「命令收到的最后一个参数」当文件读出来打到 stdout 的命令。
///
/// 注意**只写命令本身**：事件文件路径由执行器追加为最后一个词，所以这里
/// 不能自己再带占位参数（`type "%3"` 反而会读空）。跨平台只假设「最后一个词
/// 是路径」这件事，不假设具体 shell。
fn cat_last_arg_command() -> &'static str {
    if cfg!(target_os = "windows") {
        "type"
    } else {
        "cat"
    }
}

fn cmd_hook(command: impl Into<String>) -> HookConfigEntry {
    HookConfigEntry {
        name: "t".into(),
        hook_type: HookType::Command,
        command: Some(command.into()),
        url: None,
        timeout_ms: Some(30_000),
        matcher: None,
    }
}

/// 跑一个 command 类钩子，返回 `execute_single` 的原始结果。
async fn run(command: &str, event: &HookEvent, workdir: &std::path::Path) -> HookExecutionResult {
    let cfg = cmd_hook(command);
    HookExecutor::new()
        .execute_single(&cfg, event, &workdir.to_string_lossy())
        .await
}

/// `execute_single` 把 stdout折叠进 `HookOutput`（能解析 JSON 就当 JSON，否则当
/// `context`）。测试只依赖「文本被透出」这一件事，不依赖折叠的具体结构。
fn printed(out: &crate::symbio_core::HookOutput) -> String {
    out.context.clone().unwrap_or_default()
}

#[tokio::test]
async fn event_json_reaches_hook_as_trailing_argument() {
    let event = HookEvent::PreCompact;
    let json = serde_json::to_string(&event).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let res = run(cat_last_arg_command(), &event, dir.path()).await;

    assert!(res.error.is_none(), "不该报错: {:?}", res.error);
    assert_eq!(
        printed(&res.output).trim(),
        json,
        "钩子读到的载荷必须就是事件本身"
    );
}

#[tokio::test]
async fn payload_is_not_interpreted_as_shell_syntax() {
    // 载荷里带 shell 元字符。若实现把它拼进命令串，这些会被当命令执行——
    // 表现为凭空多出一个标记文件。事件只是被读出，不该有任何副作用。
    let marker: PathBuf =
        std::env::temp_dir().join(format!("symbio-hook-inject-{}", uuid::Uuid::new_v4()));
    let event = HookEvent::PreToolUse {
        tool_name: "shell".into(),
        tool_input: serde_json::json!({ "command": format!("; echo pwned > {}", marker.display()) }),
    };
    let dir = tempfile::tempdir().unwrap();

    let _ = run(cat_last_arg_command(), &event, dir.path()).await;

    assert!(
        !marker.exists(),
        "载荷里的 shell 元字符被当命令执行了（注入未消除）"
    );
}

#[tokio::test]
async fn event_file_does_not_linger_in_workdir() {
    let dir = tempfile::tempdir().unwrap();
    let _ = run(cat_last_arg_command(), &HookEvent::PreCompact, dir.path()).await;

    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("hook") || n.contains("event"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "workdir 里留下了事件文件: {leftovers:?}"
    );
}

#[tokio::test]
async fn concurrent_hooks_do_not_collide_on_event_file() {
    // 旧实现按 `<pid>.json` 命名：同进程内并发跑两个钩子会撞名，
    // 于是 A 钩子可能读到 B 的事件。这里并发跑，验证各自拿到自己的载荷。
    let dir = tempfile::tempdir().unwrap();
    let mk = |tag: &str| HookEvent::PreToolUse {
        tool_name: tag.into(),
        tool_input: serde_json::json!({ "tag": tag }),
    };

    let ea = mk("alpha");
    let eb = mk("beta");
    let (a, b) = tokio::join!(
        run(cat_last_arg_command(), &ea, dir.path()),
        run(cat_last_arg_command(), &eb, dir.path()),
    );

    for (res, tag) in [(a, "alpha"), (b, "beta")] {
        let text = printed(&res.output);
        assert!(text.contains(tag), "载荷串了：期望 {tag}，实得 {text}");
    }
}

#[tokio::test]
async fn nonzero_exit_denies_instead_of_silently_allowing() {
    let dir = tempfile::tempdir().unwrap();
    let res = run(
        if cfg!(target_os = "windows") {
            "exit /b 3"
        } else {
            "exit 3"
        },
        &HookEvent::PreCompact,
        dir.path(),
    )
    .await;

    assert!(!res.success);
    assert!(
        res.output.is_blocking(),
        "非 0/1 退出码必须收敛成 deny，不能默认放行"
    );
}

#[tokio::test]
async fn exit_code_one_is_a_warning_not_a_denial() {
    // 约定：0 = 通过，1 = 警告（仍放行），其他 = 拦截
    let dir = tempfile::tempdir().unwrap();
    let res = run(
        if cfg!(target_os = "windows") {
            "echo warn & exit /b 1"
        } else {
            "echo warn; exit 1"
        },
        &HookEvent::PreCompact,
        dir.path(),
    )
    .await;

    assert!(!res.output.is_blocking(), "退出码 1 是警告，不该拦下会话");
}

#[tokio::test]
async fn hook_can_deny_by_returning_hook_output_json() {
    // 钩子作者拦下会话的正规方式：stdout 打一份 `HookOutput` JSON，执行器把它
    // 收敛成 deny（而不是当 context 放行）。
    //
    // 关键在于命令必须**只吐那一份 JSON**：执行器会把事件文件路径追加成最后一个
    // 参数，所以不能写 `type a b`（两份拼起来解析不了）。用重定向把输出钉死在
    // 单个文件上，末尾多出的路径参数落在重定向之外、不影响 stdout。
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("out.json");
    std::fs::write(
        &payload,
        r#"{"should_proceed":false,"block_reason":"nope"}"#,
    )
    .unwrap();

    let print_only = if cfg!(target_os = "windows") {
        // 末尾的 `& rem` 吃掉执行器追加的那个路径参数（`rem` 忽略自身参数），
        // 于是 stdout 只有 payload 一份。
        format!("type {} & rem", payload.display())
    } else {
        format!("cat '{}' ; :", payload.display())
    };
    let res = run(&print_only, &HookEvent::PreCompact, dir.path()).await;

    assert!(
        res.output.is_blocking(),
        "合法 HookOutput JSON 应被解析成 deny: {:?}",
        res.output
    );
    assert_eq!(res.output.block_reason.as_deref(), Some("nope"));
}

#[tokio::test]
async fn non_hook_output_json_is_kept_as_context() {
    // 反向判据：只有 `HookOutput` 形状才收敛成 deny；别的 JSON（如事件本身）
    // 原样作为 context 交给会话，不该被误当成拦截。
    let dir = tempfile::tempdir().unwrap();
    let res = run(cat_last_arg_command(), &HookEvent::PreCompact, dir.path()).await;

    assert!(
        !res.output.is_blocking(),
        "事件 JSON 不是 HookOutput，不该拦下会话"
    );
    assert!(printed(&res.output).contains("PreCompact"));
}

#[tokio::test]
async fn timeout_is_enforced_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    // `ping` 到不可路由地址（RFC 5737 TEST-NET-1）会一直等超时，够长且确定；
    // 末尾 `& rem` / `; :` 吃掉执行器追加的路径参数。
    let cfg = HookConfigEntry {
        timeout_ms: Some(300),
        ..cmd_hook(if cfg!(target_os = "windows") {
            "ping -n 8 -w 1000 192.0.2.1 & rem".to_string()
        } else {
            "sleep 8 ; :".to_string()
        })
    };

    let res = HookExecutor::new()
        .execute_single(&cfg, &HookEvent::PreCompact, &dir.path().to_string_lossy())
        .await;

    assert!(
        res.error
            .as_deref()
            .is_some_and(|e| e.contains("timed out")),
        "超时应被如实报告: {:?}",
        res.error
    );
}

#[tokio::test]
async fn missing_command_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = HookConfigEntry {
        name: "t".into(),
        hook_type: HookType::Command,
        command: None,
        url: None,
        timeout_ms: None,
        matcher: None,
    };

    let res = HookExecutor::new()
        .execute_single(&cfg, &HookEvent::PreCompact, &dir.path().to_string_lossy())
        .await;

    assert!(!res.success);
    assert!(res.error.is_some());
}

#[tokio::test]
async fn execute_aggregates_context_across_hooks() {
    // 两个钩子各给一段 context：都该进聚合结果，彼此不覆盖
    let dir = tempfile::tempdir().unwrap();
    let cfgs = vec![cmd_hook("echo alpha"), cmd_hook("echo beta")];

    let out = HookExecutor::new()
        .execute(
            &cfgs,
            &HookEvent::PreCompact,
            "s1",
            &dir.path().to_string_lossy(),
        )
        .await;

    let ctx = out.context.unwrap_or_default();
    assert!(ctx.contains("alpha"), "{ctx}");
    assert!(ctx.contains("beta"), "{ctx}");
}

#[tokio::test]
async fn blocking_hook_stops_the_chain() {
    // deny 的钩子不该让后面的钩子继续跑——那等于拦截失效
    let dir = tempfile::tempdir().unwrap();
    let deny_file = dir.path().join("deny.json");
    std::fs::write(
        &deny_file,
        r#"{"should_proceed":false,"block_reason":"nope"}"#,
    )
    .unwrap();
    let deny = cmd_hook(if cfg!(target_os = "windows") {
        format!("type {} & rem", deny_file.display())
    } else {
        format!("cat '{}' ; :", deny_file.display())
    });
    let after = cmd_hook(if cfg!(target_os = "windows") {
        "echo should-not-appear > ran.txt".to_string()
    } else {
        "echo should-not-appear > ran.txt".to_string()
    });

    let out = HookExecutor::new()
        .execute(
            &[deny, after],
            &HookEvent::PreCompact,
            "s1",
            &dir.path().to_string_lossy(),
        )
        .await;

    assert!(out.is_blocking(), "deny 应原样透出：{out:?}");
    assert_eq!(out.block_reason.as_deref(), Some("nope"));
    assert!(
        !dir.path().join("ran.txt").exists(),
        "被拦截后不该再执行后续钩子"
    );
}

#[test]
fn default_timeout_is_sixty_seconds() {
    // `HookConfigEntry.timeout_ms` 为 None 时的回退值，改了要有人来改这条断言
    assert_eq!(HookExecutor::new().default_timeout_ms, 60_000);
}
