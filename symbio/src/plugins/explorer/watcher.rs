//! 文件监听服务
//!
//! 监听工作区目录变化，并通过事件发送器或回调函数通知

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Explorer 事件名称常量
pub const EVENT_BROWSER_DIR_CHANGED: &str = "browser/dir_changed";
pub const EVENT_BROWSER_FILE_CHANGED: &str = "browser/file_changed";

/// 文件变化事件数据
#[derive(Debug, Clone, Serialize)]
pub struct FileChangeEvent {
    pub path: String,
    pub kind: String,
    pub timestamp: u64,
}

/// 事件回调函数类型
pub type EventCallback = Arc<dyn Fn(String, serde_json::Value) + Send + Sync>;

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
    /// 创建新的文件监听器（使用事件发送器，保持兼容）
    pub fn new(event_sender: crate::symbio_core::event::OptionalEventSender) -> Self {
        Self::new_with_callback(move |event_name, payload| {
            if let Err(e) = event_sender.emit(&event_name, payload) {
                eprintln!("[watcher] Failed to emit event: {}", e);
            }
        })
    }

    /// 创建新的文件监听器（使用回调函数）
    pub fn new_with_callback<F>(callback: F) -> Self
    where
        F: Fn(String, serde_json::Value) + Send + Sync + 'static,
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
                        let event_name = if is_dir {
                            EVENT_BROWSER_DIR_CHANGED
                        } else {
                            EVENT_BROWSER_FILE_CHANGED
                        };

                        let timestamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();

                        let payload = serde_json::json!({
                            "path": file_path.to_string_lossy().to_string(),
                            "kind": format!("{:?}", event.kind),
                            "timestamp": timestamp,
                        });

                        callback(event_name.to_string(), payload);
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
