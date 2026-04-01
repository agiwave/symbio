//! Telegram 子插件模块
//!
//! 提供 Telegram Bot API 集成

pub mod plugin;
pub mod factory;
pub mod types;

pub use plugin::TelegramPlugin;
pub use factory::TelegramFactory;
