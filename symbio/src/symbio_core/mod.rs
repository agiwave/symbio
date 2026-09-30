//! 核心模块

pub mod actors;
pub mod adapters;
mod assembly;
mod capability;
mod clock;
mod creator;
mod embedding;
mod event;
mod event_bus;
pub mod exec;
pub mod governance;
mod invariants;
mod keys;
pub mod llm;
mod logger;
mod plugin;
mod projection;
pub mod schemas;
mod store;
mod text;
pub mod vdfs;
mod view;

// ==================== 事件溯源契约（v2 阶段 S0） ====================
// 共享类型层（`event` / `view`）与两个机制（`store` ④ / `projection` ③）。
// 模块依赖纪律见 [plan/05 §3.1]：彼此只准依赖类型定义，不持有对方句柄。
pub use event::{Entity, Event, EventEnvelope, Seq, Timestamp, Verb};
pub use invariants::{
    acyclic_deps, budget_exceeded, check_all, final_unique_per_turn, produced_by_coverage,
    rework_bounded, seq_monotonic, unresolved_turns, Violation,
};
pub use projection::checkpoint::{checkpoint, CheckpointState};
pub use projection::consolidate::{accept as consolidate_accept, ConsolidateParams, Rejection};
pub use projection::cost::{cost_ledger, CostEntry, CostLedgerView};
pub use projection::readyset::{readyset, ReadySetView, ReadyTask};
pub use projection::recall::recall;
pub use projection::reputation::{plain_score, reputation, ReputationEntry, ReputationView};
pub use projection::turnstate::{turnstate, TurnState};
pub use projection::Projection;
pub use store::wal::{EventWalStore, WalStore};
pub use store::{AppendError, EventStore, MemoryStore, Store};
pub use view::{Budget, View};
pub use view::{RecallEntry, RecallView};

// ==================== 主体（v2 阶段 S1，② actors） ====================
// 只收类型化输入、只产事件（plan/05 §3.1 ② 行）；S1 落 Decider 平凡值，S2 加 Reasoner。
pub use actors::{
    ActorSpec, CircuitBreaker, CommitmentKeeper, Decider, DeciderMiss, GateDecision, Pattern,
    Preemption, PreemptionDecider, Reasoner, RecallTranslator, Scope,
};
// 事件名字表（名字是数据，单点定义）。
pub use event::{
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_CONTROL_OPENED,
    EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED,
    EVENT_TASK_ASSERTED, EVENT_TASK_HELD, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
    EVENT_TASK_REWORK_CREATED, EVENT_THREAD_CHECKPOINT, EVENT_USER_MESSAGE,
};

// ==================== 适配器（v2 阶段 S2，⑤ adapters） ====================
// 时延闸门 = 本包的依赖注入策略（plan/05 §3.3）：令牌按档位签发，反射档拿不到模型句柄。
pub use adapters::{
    AdapterError, CanClassify, CanGenerate, ClassifyOnly, FullModel, LatencyTier, LlmAdapter,
    RuleOnly, StubLlmAdapter, TokenIssuer,
};

// ==================== 权限与可见性（v2 阶段 S3，⑥ governance） ====================
// 读写成对、fail-closed（plan/01 §7）。注意：governance::Capability（权限能力，
// 7 个封顶）与 capability::Capability（LLM 工具描述符）是两个概念，
// 刻意**不进根平铺**——以路径限定消歧，见 governance/mod.rs 的同名辨析。
pub use governance::{PairingViolation, PermissionMatrix, PrincipalPolicy, VisScope};

// ==================== LLM 契约 ====================
// 模型接入（`model_provider`）与单轮产物 / 帧原语（`turn`）。
// 行解析契约与协议事件方言只有 model 插件使用，住在 `plugins/model/protocols/`。
pub use llm::model_provider::{ModelFinishReason, ModelProvider, ModelUsage};
pub use llm::turn::{
    llm_emit_message, llm_message_frame, llm_removed_frame, llm_short_id, TurnOutput,
    TurnToolCallInfo,
};

// ==================== VDFS 契约 ====================
// 统一资源访问面的全部公开符号（定义汇聚于 `vdfs` 模块）。
// 词表常量：状态 / 基础类型 / 呈现扩展名 / 节点动作 / 调用级参数 / 注册表字段名
pub use vdfs::{
    VDFS_ACTION_ABORT, VDFS_ACTION_CLEAR, VDFS_ACTION_DISABLE, VDFS_ACTION_ENABLE,
    VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT, VDFS_ACTION_PLUGINS, VDFS_ACTION_TEST,
    VDFS_ACTION_TRUNCATE, VDFS_EXT_DIR, VDFS_EXT_FORM, VDFS_EXT_JSON, VDFS_EXT_MARKDOWN,
    VDFS_EXT_TEXT, VDFS_EXT_ZIP, VDFS_KIND_DIR, VDFS_KIND_FILE, VDFS_PARAM_BEFORE,
    VDFS_PARAM_LIMIT, VDFS_PARAM_WORKDIR, VDFS_PLUGINS_FIELD, VDFS_PLUGIN_NAME_FIELD,
    VDFS_PLUGIN_PROVIDER_FIELD, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED, VDFS_STATUS_FAILED,
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
    vdfs_change_of, vdfs_context, vdfs_derive_ext, vdfs_from_plugin_error, vdfs_has_parent_segment,
    vdfs_host_ctx, vdfs_notify_change, vdfs_path_within, vdfs_unwatch_changes, vdfs_watch_changes,
    VdfsChangeSubscriptions,
};
// 地址机制（crate 内部装配用，不是公开契约）
pub(crate) use vdfs::{absolute_addr, descend_addr, join_addr, AddrRootDecl};

// ==================== 事件总线 ====================
pub use event_bus::{
    event_bus_build_envelope, event_bus_register_subscriber, event_bus_unregister_subscriber,
    EventBus, EventBusSubscribeRequest, EVENT_BUS_KIND_SYSTEM, EVENT_BUS_KIND_VDFS,
    EVENT_BUS_RESYNC_MARKER_TYPE,
};

// ==================== 服务抽象 ====================
pub use embedding::{EmbeddingError, EmbeddingService, EMBEDDING_LOCAL, EMBEDDING_NOOP};

// ==================== 插件契约 ====================
pub use capability::{
    capability_announce_configurable, capability_entry_of, capability_invoke, capability_resolve,
    capability_to_wire, Capability, CapabilityCategory, CapabilityMeta,
    CapabilityToolContextRetention, CapabilityVisitor, ConfigurableVisitor, OptionVisitor,
};
// 工具结果 `failure_kind` 闭集：生产方（`local`）与消费方（`session`）分属不同插件，
// 互相不可见，只能经这里共享。单独一行——它是模块而非类型。
pub use capability::failure_kind;
pub use plugin::*;
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录

// ==================== 通用对象创建注册表 ====================
// 按 id 装配任意类型对象——**不是插件专属**：注册表按 `(id, TypeId)` 索引，
// 当前服务 `dyn Plugin` / `dyn ModelProtocol` / `dyn EmbeddingService` 三个类型族。
// 独立成域的理由见 `creator/mod.rs` 的模块文档。
pub use creator::{creator_create_object, creator_has, creator_ids};
// 注册表内部结构，供 `submit_object_creator!` 宏展开取用（crate 内可见）
pub(crate) use creator::{ObjectConstructor, Submit};

// ==================== 键面 ====================
pub use keys::*;

// ==================== 装配策略 ====================
// 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」——不是键、不是 id，见 assembly 模块文档。
pub use assembly::{ASSEMBLY_SUB_AGENT_PLUGINS, ASSEMBLY_UNDISABLABLE_PLUGINS};

// ==================== 基础设施 / 工具函数 ====================
// 注：`CAPABILITY_ERRORS`（错误桶键）不在这里——它是 `SymbioKey` 实例，
// 随 `pub use keys::*` 一并平铺（键面只有一个定义处，见 `keys/mod.rs`）。
pub use capability::{
    capability_init_error_bucket, capability_report_error, capability_take_errors, CapabilityError,
};
pub use clock::clock_now_ms;
// 锁辅助函数**刻意不进 `pub use plugin::*`**（见 `plugin/error.rs::lock_read` 的说明）：
// 显式 `pub(crate)` 导入，既让全 crate 可用，又保留 `dead_code` 的可见性。
pub use exec::{
    ExecAbortSignal, ExecEnv, ExecEventSink, ExecEventSinkProgress, ExecTranscriptWriter,
};
// 注：homedir（系统根注册表）**不在 core**——它归 `home` 插件独有。core 只提供
// 纯路径工具 `plugin_expand_tilde_path`（经 `plugin::dir` 重导出，不读任何全局系统根）。
pub use logger::*;
// 注：记忆（`MemoryFile` / 两道闸门 / 片段渲染 / 节点形状）**整块不在 core**——
// 实现不隶属任何单个插件（memory / session 各用一个或多个作用域），故归
// `providers/memory`（方式 B，不套 `dyn`）。**连文件名也不在 core**：各作用域
// 定义自己的文件名常量（`memory::memory::MEMORY_FILE` 等），core 不设统一约定。
pub(crate) use plugin::{lock_read, lock_write};
pub use text::{text_floor_char_boundary, text_truncate_bytes};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;
