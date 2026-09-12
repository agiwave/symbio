//! 核心模块

mod capability;
mod chat_pipeline;
mod chat_session;
pub(crate) mod creator;
pub mod entities;
mod error;
pub mod event_bus;
mod homedir;
mod ids;
mod keys;
mod logger;
pub mod model_provider;
pub mod option;
mod paths;
mod plugin;
pub mod providers;
pub mod schemas;
mod text;
mod tools;
mod transport;
pub mod turn;

pub use chat_pipeline::{
    attach_capabilities, collect_capabilities, report_error, take_errors, CapabilityError,
    CAPABILITY_ERRORS,
};
pub use chat_session::{ChatSession, ChatSessionHandle};
pub use creator::{create_object, has_creator};
pub use model_provider::{FinishReason, ModelProvider, ProtocolEvent, Usage};
pub use option::{
    collect_options, DefaultOptionVisitor, OptionVisitor, TRAVERSE_AVAILABLE_OPTIONS,
};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录
pub use capability::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityVisitor, ToolContextRetention,
};
pub use error::*;
pub use homedir::{expand_tilde_path, HomedirRegistry, DEFAULT_HOMEDIR};
pub use ids::*;
pub use keys::*;
pub use logger::*;
pub use paths::*;
pub use plugin::*;
pub use text::{floor_char_boundary, truncate_bytes};
pub use tools::DefaultToolVisitor;
pub use transport::{
    PluginChannel, PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire,
};
pub use turn::{
    build_assistant_messages, build_tool_message, emit_abort, emit_status, emit_update,
    execute_post_with_abort, get_http_client, parse_sse_stream, short_id,
    try_parse_partial_sse_line, PostResult, StreamChildIds, ToolCallAccumulator, ToolCallInfo,
    TurnOutput,
};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;

/// 遍历可用工具的常量路径
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";
