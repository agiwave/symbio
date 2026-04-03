//! Explorer 插件模块 - 工作区资源浏览器

mod plugin;
mod factory;
mod watcher;

pub use plugin::ExplorerPlugin;
pub use factory::ExplorerFactory;
pub use watcher::{FileWatcher, EVENT_BROWSER_DIR_CHANGED, EVENT_BROWSER_FILE_CHANGED};
