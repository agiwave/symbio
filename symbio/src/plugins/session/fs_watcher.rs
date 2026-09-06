//! 文件系统监听（基础设施）
//!
//! 递归监听一个目录及其子树，文件/目录变化时以**绝对路径**回调。
//! 事件的去重、语义化（如「容器子实体数据变更」粗粒度事件）由使用方
//! （provider 场景层）负责，本模块不感知任何业务语义。

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// 变化回调：收到变化的条目绝对路径
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

    /// 开始递归监听目录
    pub async fn start(&self, path: PathBuf) -> Result<(), String> {
        let callback = self.event_callback.clone();
        let watcher_ref = self.watcher.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    for file_path in &event.paths {
                        callback(file_path.clone());
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
