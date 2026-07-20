//! 文件监听服务
//!
//! 监听工作区目录变化，并通过事件发送器或回调函数通知

use crate::symbio_core::schemas::explorer::explorer_event;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// 事件回调函数类型
pub type EventCallback =
    Arc<dyn Fn(explorer_event::ExplorerEventType, serde_json::Value) + Send + Sync>;

/// 文件监听器
pub struct FileWatcher {
    watcher: Arc<RwLock<Option<RecommendedWatcher>>>,
    event_callback: EventCallback,
}

impl Clone for FileWatcher {
    fn clone(&self) -> Self {
        Self {
            watcher: self.watcher.clone(),
            event_callback: self.event_callback.clone(),
        }
    }
}

impl FileWatcher {
    /// 创建新的文件监听器（使用回调函数）
    pub fn new_with_callback<F>(callback: F) -> Self
    where
        F: Fn(explorer_event::ExplorerEventType, serde_json::Value) + Send + Sync + 'static,
    {
        Self {
            watcher: Arc::new(RwLock::new(None)),
            event_callback: Arc::new(callback),
        }
    }

    /// 开始监听目录
    pub async fn start(&self, path: PathBuf) -> Result<(), String> {
        let callback = self.event_callback.clone();
        let watcher_ref = self.watcher.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    for file_path in &event.paths {
                        let is_dir = file_path.is_dir();
                        let event_type = if is_dir {
                            explorer_event::ExplorerEventType::DirChanged
                        } else {
                            explorer_event::ExplorerEventType::FileChanged
                        };

                        let input = serde_json::to_value(explorer_event::Event::FileChange {
                            path: file_path.to_string_lossy().to_string(),
                            kind: format!("{:?}", event.kind),
                        })
                        .unwrap_or_default();

                        callback(event_type, input);
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
