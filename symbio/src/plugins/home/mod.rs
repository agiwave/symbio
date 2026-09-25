//! Home 插件模块 - 根插件，持有 work/agent/plugin_manager 子插件
//!
//! `homedir` 子模块是**全项目唯一**知道系统根的地方（见其文档）：普通插件
//! 一律只认父插件经 `PLUGIN_DIR` 告知的自己的目录。

mod homedir;
mod plugin;
mod schemas;
