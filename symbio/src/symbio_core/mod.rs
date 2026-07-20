//! 核心模块

mod capability;
mod chat_session;
pub(crate) mod creator;
mod error;
pub mod event_bus;
mod homedir;
mod ids;
mod keys;
mod logger;
mod paths;
mod plugin;
pub mod providers;
pub mod schemas;
mod system;
mod transport;
mod types;

pub use chat_session::{ChatSession, ChatSessionHandle};
pub use creator::{create_object, has_creator};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录
pub use capability::{Capability, CapabilityCategory, CapabilityManager, CapabilityMeta};
pub use error::*;
pub use homedir::{expand_tilde_path, HomedirRegistry, DEFAULT_HOMEDIR};
pub use ids::*;
pub use keys::*;
pub use logger::*;
pub use paths::*;
pub use plugin::*;
pub use system::{decode_output, run_command, validate_params};
pub use transport::{PluginChannel, PluginFrame, PluginPayload};
pub use types::{BoxStream, EventResult, SystemEvent, ToolCall};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;

/// 遍历可用工具的常量路径
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";
