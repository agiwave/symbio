//! 文件监听服务
//!
//! 监听工作区目录变化，并通过 Tauri 事件通知前端

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::RwLock;

/// 事件名称常量（与系统架构一致）
pub const EVENT_BROWSER_DIR_CHANGED: &str = "browser/dir_changed";
pub const EVENT_BROWSER_FILE_CHANGED: &str = "browser/file_changed";

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
    app_handle: tauri::AppHandle,
}

impl Clone for FileWatcher {
    fn clone(&self) -> Self {
        Self {
            watcher: self.watcher.clone(),
            app_handle: self.app_handle.clone(),
        }
    }
}

impl FileWatcher {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self {
            watcher: Arc::new(RwLock::new(None)),
            app_handle,
        }
    }

    /// 开始监听目录
    pub async fn start(&self, path: PathBuf) -> Result<(), String> {
        let app = self.app_handle.clone();
        let watcher_ref = self.watcher.clone();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    for path in &event.paths {
                        let is_dir = path.is_dir();
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
                            path: path.to_string_lossy().to_string(),
                            kind: format!("{:?}", event.kind),
                            timestamp,
                        };

                        if let Err(e) = app.emit(event_name, payload) {
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
