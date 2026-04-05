//! 文件监听服务
//!
//! 监听工作区目录变化，并通过事件发送器通知外部

use crate::symbio_core::event::OptionalEventSender;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Explorer 事件名称常量
const EVENT_BROWSER_DIR_CHANGED: &str = "browser/dir_changed";
const EVENT_BROWSER_FILE_CHANGED: &str = "browser/file_changed";

/// 文件变化事件数据
#[derive(Debug, Clone, Serialize)]
pub struct FileChangeEvent {
    pub path: String,
    pub kind: String,
    pub timestamp: u64,
}

/// 文件监听器
pub struct FileWatcher {
    watcher: Arc<RwLock<Option<RecommendedWatcher>>>,
    event_sender: OptionalEventSender,
}

impl Clone for FileWatcher {
    fn clone(&self) -> Self {
        Self {
            watcher: self.watcher.clone(),
            event_sender: self.event_sender.clone(),
        }
    }
}

impl FileWatcher {
    pub fn new(event_sender: OptionalEventSender) -> Self {
        Self {
            watcher: Arc::new(RwLock::new(None)),
            event_sender,
        }
    }

    /// 开始监听目录
    pub async fn start(&self, path: PathBuf) -> Result<(), String> {
        let event_sender = self.event_sender.clone();
        let watcher_ref = self.watcher.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    for file_path in &event.paths {
                        let is_dir = file_path.is_dir();
                        let event_name = if is_dir {
                            EVENT_BROWSER_DIR_CHANGED
                        } else {
                            EVENT_BROWSER_FILE_CHANGED
                        };

                        let timestamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();

                        let payload = FileChangeEvent {
                            path: file_path.to_string_lossy().to_string(),
                            kind: format!("{:?}", event.kind),
                            timestamp,
                        };

                        if let Err(e) = event_sender.emit(event_name, payload) {
                            eprintln!("[watcher] Failed to emit event: {}", e);
                        }
                    }
                }
            },
            Config::default()
        )
        .map_err(|e| format!("Failed to create watcher: {}", e))?;

        watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch path: {}", e))?;

        let mut w = watcher_ref.write().await;
        *w = Some(watcher);

        eprintln!("[watcher] Started watching: {}", path.display());
        Ok(())
    }

    /// 停止监听
    pub async fn stop(&self) {
        let mut w = self.watcher.write().await;
        *w = None;
        eprintln!("[watcher] Stopped watching");
    }

    /// 重新加载监听（用于切换工作区时）
    pub async fn reload(&self, new_path: PathBuf) -> Result<(), String> {
        self.stop().await;
        self.start(new_path).await
    }
}
