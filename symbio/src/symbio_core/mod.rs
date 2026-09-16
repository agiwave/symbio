//! 核心模块

mod capability;
mod capability_error;
mod clock;
mod configurable;
pub(crate) mod creator;
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
mod plugin_dir;
pub mod providers;
pub mod schemas;
mod text;
mod tools;
mod transport;
pub mod turn;
pub mod vdfs;
pub mod vdfs_provider;

pub use capability_error::{
    init_error_bucket, report_error, take_errors, CapabilityError, CAPABILITY_ERRORS,
};
pub use creator::{create_object, has_creator};
pub use model_provider::{FinishReason, ModelProvider, ProtocolEvent, Usage};
pub use option::{
    collect_options, DefaultOptionVisitor, OptionVisitor, TRAVERSE_AVAILABLE_OPTIONS,
};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录
pub use capability::{
    Capability, CapabilityCategory, CapabilityMeta, CapabilityVisitor, ToolContextRetention,
};
pub use clock::now_ms;
pub use configurable::{
    announce_configurable, entry_of, ConfigurableVisitor, DefaultConfigurableVisitor,
};
pub use error::*;
pub use homedir::{expand_tilde_path, HomedirRegistry, DEFAULT_HOMEDIR};
pub use ids::*;
pub use keys::*;
pub use logger::*;
pub use paths::*;
pub use plugin::*;
pub use plugin_dir::{
    config_file_of, dir_from_ctx, dir_of, plugins_root, ConfigFile, PluginDir, KEY_NAME,
    KEY_PROVIDER, PLUGINS_DIR, PLUGIN_FILE,
};
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
pub use vdfs::{
    DynVdfsProvider, VdfsAccess, VdfsChange, VdfsContent, VdfsError, VdfsNode, VdfsProvider,
    VdfsValidationError,
};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;

/// 遍历可用工具的常量路径
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";
