//! 核心模块

mod assembly;
mod capability;
mod clock;
mod embedding;
pub mod event_bus;
pub mod exec;
mod keys;
pub mod llm;
mod logger;
mod memory;
mod plugin;
pub mod schemas;
mod text;
pub mod vdfs;

// ==================== LLM 契约 ====================
// 模型接入（`model_provider`）与单轮产物 / 帧原语（`turn`）。
// 行解析契约与协议事件方言只有 model 插件使用，住在 `plugins/model/protocols/`。
pub use llm::model_provider::{ModelFinishReason, ModelProvider, ModelUsage};
pub use llm::turn::{
    build_assistant_messages, build_tool_message, emit_delta, emit_message, emit_removed,
    emit_state, message_frame, removed_frame, short_id, state_frame, TurnOutput,
    TurnStreamChildIds, TurnToolCallAccumulator, TurnToolCallInfo,
};

// ==================== VDFS 契约 ====================
// 统一资源访问面的全部公开符号（定义汇聚于 `vdfs` 模块）。
// 词表常量：状态 / 基础类型 / 呈现扩展名 / 节点动作 / 调用级参数 / 注册表字段名
pub use vdfs::{
    PLUGIN_PROVIDER_FIELD, VDFS_ACTION_ABORT, VDFS_ACTION_CLEAR, VDFS_ACTION_DISABLE,
    VDFS_ACTION_ENABLE, VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT, VDFS_ACTION_PLUGINS,
    VDFS_ACTION_TEST, VDFS_ACTION_TRUNCATE, VDFS_EXT_DIR, VDFS_EXT_FORM, VDFS_EXT_JSON,
    VDFS_EXT_MARKDOWN, VDFS_EXT_TEXT, VDFS_EXT_ZIP, VDFS_KIND_DIR, VDFS_KIND_FILE,
    VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT, VDFS_PARAM_WORKDIR, VDFS_PLUGINS_FIELD,
    VDFS_PLUGIN_NAME_FIELD, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED, VDFS_STATUS_FAILED,
    VDFS_STATUS_NONE, VDFS_STATUS_UNKNOWN, VDFS_STATUS_WORKING,
};
// 域类型：节点 / 内容 / 请求响应 / 错误 / 变更
pub use vdfs::{
    DynVdfsProvider, VdfsAccess, VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent,
    VdfsContext, VdfsError, VdfsFieldError, VdfsItem, VdfsNewType, VdfsNode, VdfsParams,
    VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsValidationError, VdfsWriteResponse,
};
// 契约函数与宿主桥
pub use vdfs::{
    derive_ext, from_plugin_error, has_parent_segment, host_ctx, notify_change, path_within,
    unwatch_changes, vdfs_change_of, vdfs_context, watch_changes, ChangeSubscriptions,
};
// 地址机制（crate 内部装配用，不是公开契约）
pub(crate) use vdfs::{absolute_addr, descend_addr, join_addr, AddrRootDecl};

// ==================== 事件总线 ====================
pub use event_bus::{
    build_envelope, register_subscriber, unregister_subscriber, EventBus, EventBusSubscribeRequest,
    EVENT_BUS_KIND_SYSTEM, EVENT_BUS_KIND_VDFS,
};

// ==================== 服务抽象 ====================
pub use embedding::{EmbeddingError, EmbeddingService};

// ==================== 插件契约 ====================
pub use capability::{
    announce_configurable, collect_options, entry_of, invoke_capability, resolve, to_wire,
    Capability, CapabilityCategory, CapabilityMeta, CapabilityToolContextRetention,
    CapabilityVisitor, ConfigurableVisitor, DefaultConfigurableVisitor, DefaultOptionVisitor,
    DefaultToolVisitor, OptionVisitor, TRAVERSE_AVAILABLE_OPTIONS,
};
// 工具结果 `failure_kind` 闭集：生产方（`local`）与消费方（`session`）分属不同插件，
// 互相不可见，只能经这里共享。单独一行——它是模块而非类型。
pub use capability::failure_kind;
pub use plugin::*;
// 注册表内部结构，供 `submit_object_creator!` 宏展开取用（crate 内可见）
pub(crate) use plugin::{ObjectConstructor, Submit};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录

// ==================== 键面 ====================
pub use keys::*;

// ==================== 装配策略 ====================
// 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」——不是键、不是 id，见 assembly 模块文档。
pub use assembly::{SUB_AGENT_PLUGINS, UNDISABLABLE_PLUGINS};

// ==================== 基础设施 / 工具函数 ====================
pub use capability::{
    init_error_bucket, report_error, take_errors, CapabilityError, CAPABILITY_ERRORS,
};
pub use clock::now_ms;
// 锁辅助函数**刻意不进 `pub use plugin::*`**（见 `plugin/error.rs::lock_read` 的说明）：
// 显式 `pub(crate)` 导入，既让全 crate 可用，又保留 `dead_code` 的可见性。
pub use exec::{
    ExecAbortSignal, ExecEnv, ExecEventSink, ExecEventSinkProgress, ExecTranscriptWriter,
};
// 注：homedir（系统根注册表）**不在 core**——它归 `home` 插件独有。core 只提供
// 纯路径工具 `expand_tilde_path`（经 `plugin::dir` 重导出，不读任何全局系统根）。
pub use logger::*;
pub use memory::{
    render_segment, MemoryFile, MemoryInjection, MemoryNodeSpec, MemorySegmentSpec,
    MEMORY_AGENTS_FILE,
};
pub(crate) use plugin::{lock_read, lock_write};
pub use text::{floor_char_boundary, truncate_bytes};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;

/// 遍历可用工具的常量路径
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";
