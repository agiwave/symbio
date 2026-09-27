//! `fs_watcher` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `fs_watcher.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use tempfile::TempDir;

/// 回归：notify 的回调运行在专属 OS 线程上（非 tokio runtime 上下文），
/// 此前使用方在回调里直接 `tokio::spawn` → panic 穿越 FFI → 进程 abort。
/// 契约：回调体必须已经过 Handle 派发、在 runtime 上下文中执行。
#[tokio::test]
async fn callback_runs_in_runtime_and_receives_paths() {
    let tmp = TempDir::new().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let watcher = FsWatcher::new_with_callback(move |p| {
        assert!(
            Handle::try_current().is_ok(),
            "FsWatcher 回调未在 runtime 上下文中执行"
        );
        let _ = tx.send(p);
    });
    watcher.start(tmp.path().to_path_buf()).await.unwrap();

    // notify 启动异步，稍候再产生变化
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    tokio::fs::write(tmp.path().join("a.txt"), "x")
        .await
        .unwrap();

    let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await;
    assert!(got.is_ok(), "未在超时内收到文件变化回调");
}
