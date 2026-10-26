//! stdio 优雅关闭的跨平台回归测试：真实子进程 + stdin EOF。
//!
//! 子进程机制：测试二进制以自身重启（`current_exe()` + libtest 过滤器
//! `eof_child_mode` + 环境变量标记），子进程里只有 `eof_child_mode`
//! 一个测试会运行——它阻塞读 stdin 直到 EOF，然后正常退出（码 0）。
//! 不依赖 shell / Python / 网络服务，Windows 与 Unix 行为一致。

use super::*;

/// 子进程模式环境变量标记。
const EOF_CHILD_ENV: &str = "SYMBO_STDIO_EOF_CHILD";

/// 子进程入口：阻塞读 stdin 至 EOF 后正常返回（退出码 0）。
/// 在父进程的常规测试运行中（无环境变量标记）这是空跑。
#[test]
fn eof_child_mode() {
    if std::env::var(EOF_CHILD_ENV).is_err() {
        return;
    }
    use std::io::Read;
    let mut buf = Vec::new();
    // 阻塞直到 stdin EOF——这正是被测 shutdown 逻辑依赖的退出信号。
    let _ = std::io::stdin().read_to_end(&mut buf);
}

fn spawn_eof_child() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(std::env::current_exe().unwrap());
    cmd.arg("eof_child_mode") // libtest 过滤器：子进程只跑这一个测试
        .env(EOF_CHILD_ENV, "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd
}

#[tokio::test]
async fn eof_child_fixture() {
    // 夹具自检：子进程在 stdin EOF 前保持存活，收到 EOF 后以 0 退出。
    let mut child = spawn_eof_child().spawn().unwrap();
    assert!(
        child.try_wait().unwrap().is_none(),
        "子进程不应在 EOF 前退出"
    );
    drop(child.stdin.take()); // 发送 EOF
    let status = child.wait().await.unwrap();
    assert!(status.success(), "EOF 子进程应正常退出: {status:?}");
}

#[tokio::test]
async fn graceful_shutdown_allows_eof_child_to_exit_successfully() {
    // 行为保护测试（并非修复前失败的缺陷复现）：Tokio wait 自动关闭 stdin。
    // EOF 型子进程应在优雅窗口内自行退出、退出码 0，而不是等满超时被强杀。
    let mut child = spawn_eof_child().spawn().unwrap();
    assert!(
        child.try_wait().unwrap().is_none(),
        "子进程不应在 EOF 前退出"
    );
    let started = std::time::Instant::now();
    shutdown_child_graceful(&mut child).await;
    let status = child.wait().await.unwrap();
    assert!(
        status.success(),
        "EOF 子进程应在优雅窗口内自行退出而非被强杀: {status:?}"
    );
    // 检查正常 EOF 退出没有走到超时回收分支。
    assert!(
        started.elapsed() < GRACEFUL_SHUTDOWN_TIMEOUT,
        "shutdown 不应等待到超时（耗时 {:?}）",
        started.elapsed()
    );
}
