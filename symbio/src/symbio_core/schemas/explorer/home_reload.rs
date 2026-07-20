//! home/reload 路由的请求/响应 schema
//!
//! 对应 HomePlugin::route 中 "reload" 分支。

use serde::{Deserialize, Serialize};

/// home/reload 请求
///
/// - `homedir`：可选，要切换到的目标 homedir（绝对路径或 `~` 前缀）。
///   若不传，仅重新加载当前 homedir 的 config（用于 bootstrap 恢复后的二次重建）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Request {
    /// 目标 homedir（可选）
    #[serde(default)]
    pub homedir: Option<String>,
}

/// home/reload 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// 切换前的 homedir
    pub old_homedir: String,
    /// 切换后的 homedir
    pub new_homedir: String,
    /// 重新构造的子插件数量
    pub reloaded_plugins: usize,
    /// 是否实际发生了 homedir 切换（false 表示仅重新加载）
    pub homedir_changed: bool,
    /// bootstrap 文件路径
    pub bootstrap_path: String,
}
