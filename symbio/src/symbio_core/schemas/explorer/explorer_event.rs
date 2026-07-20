use serde::{Deserialize, Serialize};

/// 资源管理器事件类型
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    #[serde(rename = "file_change")]
    FileChange {
        path: String,
        kind: String, // "create", "modify", "remove", "rename", "other"
    },
    #[serde(rename = "watcher_error")]
    WatcherError { message: String },
}

/// 资源管理器通知类型
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerEventType {
    #[serde(rename = "browser/dir_changed")]
    DirChanged,
    #[serde(rename = "browser/file_changed")]
    FileChanged,
    #[serde(rename = "watch_started")]
    WatchStarted,
    #[serde(rename = "watch_stopped")]
    WatchStopped,
    #[serde(rename = "list_result")]
    ListResult,
}

/// 事件包裹结构（符合 Data 帧结构）
#[derive(Debug, Serialize, Deserialize)]
pub struct EventInput {
    pub r#type: ExplorerEventType,
    pub data: Event,
}

/// 生命周期事件包裹结构（用于 watch_started 等不需要 Event 数据的场景）
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusInput {
    pub r#type: ExplorerEventType,
    pub data: serde_json::Value,
}
