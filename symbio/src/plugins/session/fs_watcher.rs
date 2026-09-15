//! 文件系统监听（基础设施）
//!
//! 递归监听一个目录及其子树，文件/目录变化时以**绝对路径**回调。
//! 事件的去重、语义化（如「容器子条目变更」粗粒度事件）由使用方
//! （provider 场景层）负责，本模块不感知任何业务语义。
//!
//! ## 线程模型（关键约束）
//!
//! notify 的事件回调运行在它自己的**专属 OS 线程**上——不在 tokio runtime
//! 上下文内，该线程上直接调用 `tokio::spawn` 会 panic（且 panic 穿越
//! FFI 边界会直接 abort 进程）。因此 `start` 在 runtime 内捕获
//! [`Handle`](tokio::runtime::Handle)，回调统一经 Handle 派发回 runtime
//! 执行，使用方的异步逻辑（sleep / spawn 等）得以正常工作。

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tracing::info;

/// 变化回调：收到变化的条目绝对路径（在 tokio runtime 上下文中执行）
pub type FsEventCallback = Arc<dyn Fn(PathBuf) + Send + Sync>;

/// 文件监听器（每实例绑定一个目录，`start` 后持有句柄直至 drop）
#[derive(Clone)]
pub struct FsWatcher {
    watcher: Arc<RwLock<Option<RecommendedWatcher>>>,
    event_callback: FsEventCallback,
}

impl FsWatcher {
    pub fn new_with_callback<F>(callback: F) -> Self
    where
        F: Fn(PathBuf) + Send + Sync + 'static,
    {
        Self {
            watcher: Arc::new(RwLock::new(None)),
            event_callback: Arc::new(callback),
        }
    }

    /// 开始递归监听目录（必须在 tokio runtime 内调用）
    pub async fn start(&self, path: PathBuf) -> Result<(), String> {
        // notify 回调线程不在 runtime 内：在此捕获 Handle 供其派发
        let handle = Handle::try_current()
            .map_err(|e| format!("FsWatcher::start 必须在 tokio runtime 内调用: {e}"))?;
        let callback = self.event_callback.clone();
        let watcher_ref = self.watcher.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    for file_path in &event.paths {
                        let p = file_path.clone();
                        let cb = callback.clone();
                        // 派发回 runtime：回调体（含使用方的 spawn/sleep）在
                        // runtime 上下文中执行
                        handle.spawn(async move { cb(p) });
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| format!("Failed to create watcher: {e}"))?;

        watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch path: {e}"))?;

        let mut w = watcher_ref.write().await;
        *w = Some(watcher);

        info!(path = %path.display(), "Started watching");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
