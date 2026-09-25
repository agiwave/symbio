//! 核心模块

mod capability;
mod capability_error;
mod clock;
mod configurable;
pub(crate) mod creator;
mod error;
pub mod event_bus;
pub mod exec;
mod homedir;
mod ids;
mod keys;
mod logger;
mod memory;
pub mod model_provider;
pub mod option;
mod paths;
mod plugin;
mod plugin_dir;
pub mod providers;
pub mod schemas;
pub mod sse;
mod text;
pub mod tool_name;
mod tools;
mod transport;
pub mod turn;
pub mod vdfs;
pub mod vdfs_provider;

pub use capability_error::{
    init_error_bucket, report_error, take_errors, CapabilityError, CAPABILITY_ERRORS,
};
pub use creator::{create_object, creator_ids, has_creator};
pub use memory::{render_segment, InjectedMemory, MemoryFile, NodeSpec, SegmentSpec, AGENTS_FILE};
pub use model_provider::{FinishReason, ModelProvider, ProtocolEvent, Usage};
pub use option::{
    collect_options, DefaultOptionVisitor, OptionVisitor, TRAVERSE_AVAILABLE_OPTIONS,
};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录
pub use capability::{
    invoke_capability, Capability, CapabilityCategory, CapabilityMeta, CapabilityVisitor,
    ToolContextRetention,
};
// 工具结果 `failure_kind` 闭集：生产方（`local`）与消费方（`session`）分属不同插件，
// 互相不可见，只能经这里共享。单独一行——它是模块而非类型。
pub use capability::failure_kind;
pub use clock::now_ms;
pub use configurable::{
    announce_configurable, entry_of, ConfigurableVisitor, DefaultConfigurableVisitor,
};
pub use error::*;
pub use exec::{AbortSignal, EventSink, EventSinkProgress, ExecEnv, TranscriptWriter};
// 锁辅助函数**刻意不走 `pub use error::*`**（见 `error.rs::lock_read` 的说明）：
// 显式 `pub(crate)` 导入，既让全 crate 可用，又保留 `dead_code` 的可见性。
pub(crate) use error::{lock_read, lock_write};
pub use homedir::{expand_tilde_path, HomedirRegistry, DEFAULT_HOMEDIR};
pub use ids::*;
pub use keys::*;
pub use logger::*;
pub use paths::*;
pub use plugin::*;
pub use plugin_dir::{
    config_file_of, dir_from_ctx, dir_of, plugins_root, ConfigFile, PluginDir, PluginEntry,
    KEY_CAN_DISABLE, KEY_ENABLED, KEY_NAME, KEY_PROVIDER, KEY_REQUIRED, KEY_VERSION, PLUGIN_FILE,
};
pub use sse::{PartialLineExtractor, SseLineParser};
pub use text::{floor_char_boundary, truncate_bytes};
pub use tools::DefaultToolVisitor;
pub use transport::{
    PluginChannel, PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire,
};
pub use turn::{
    build_assistant_messages, build_tool_message, emit_delta, emit_message, emit_removed,
    emit_state, execute_post_with_abort, get_http_client, parse_sse_stream, removed_frame,
    short_id, state_frame, PostResult, StreamChildIds, ToolCallAccumulator, ToolCallInfo,
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
