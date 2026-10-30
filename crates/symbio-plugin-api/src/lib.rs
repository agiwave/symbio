//! 插件 API —— **扩展层插件的唯一宿主依赖**（DE-023 D-253）。
//!
//! 两类东西住在这里：
//!
//! **一、端口与词汇**（本 crate 自己拥有、或按清单生成）——它们不是机制实现，是插件与
//! 宿主共用的词：
//!
//! - [`routes`]：跨插件调用路径（地址）常量
//! - [`events`]：跨插件事件名常量与共享事件行（写闸的共享臂）
//! - [`behaviors`]：能力自声明的行为名（内核只装开放集合，不认识名字）
//! - [`effects`]：能力效果的实现端口（`ToolBehavior`）与草稿（`FactDraft`）
//! - [`rows`]：能力行的执行体端口（`RowHandler`）与债实例（`RowRequest`）
//! - [`context`]：能力贡献每轮上下文的端口（`ContextProvider`）与段身份
//! - [`settle`]：收束之后贡献派生事实的端口（`Contributor`）
//! - [`hooks`]：`fire_hook` 助手与 hook 事件枚举
//! - [`declarations`]：声明表的链接期提交口（事件 / 事实 / 视图 / 端口 / 执行体 / 行）
//! - [`views`]：声明视图的跨插件读口（按 owner 公开的名字读纯折叠）
//! - [`view_names`]：`shared_views` 清单生成物（哪些视图公开可读）
//!
//! **二、扩展层宿主面白名单**（文件末尾的 `pub use`）——插件要的语法类型、机制入口与
//! 装配常量，逐条转发、逐条可审。按**来源**分三臂（D-255 / D-258）：
//!
//! | 臂 | 来源 | 性质 |
//! |---|---|---|
//! | 语法臂 | `symbio-kernel` | 形状与闭集，**不计债** |
//! | SDK 臂 | `symbio-sdk` | 只碰内核词汇的宿主机制，**不计债** |
//! | core 臂 | `symbio-core` | 还欠着的机制（认识容器概念），条数只许降 |
//!
//! 插件因此只写 `symbio_plugin_api::…`，manifest 里也不列 `symbio-core` /
//! `symbio-kernel` / `symbio-sdk` / `symbio-deploy`；控制面（`loop` / `session`）
//! 不在此列，它们按影响类运行、本来就认识宿主的容器概念（D-253）。判据是
//! `plugin-surface-audit`（HS-001 零直接依赖 / HS-002 core 臂只许降 / HS-003 无空转条目）。
//!
//! 插件不得依赖宿主 crate（`symbio`）——那会形成「插件 ← 宿主 ← 插件」的环。
//! `declarations` 只依赖 `symbio_core` 的声明容器，不认识任何具体插件；收集在链接期
//! 发生，因此插件之间仍然零依赖（DE-024 D-198）。

pub mod behaviors;
pub mod context;
pub mod declarations;
pub mod effects;
pub mod events;
pub mod hooks;
pub mod routes;
pub mod rows;
pub mod settle;
pub mod view_names;
pub mod views;

pub use context::{
    fit_sections, providers as context_providers, section_meta, sections as context_sections,
    Cadence, ContextProvider, ContextRequest, ContextSection, DelegateVerdict, LiveInputs,
    RecallFacts, RecallLine, SectionFit,
};
pub use declarations::{
    assemble_declarations, declared_event_cell, declared_vocab, sealed_declarations, vocab,
    DeclarationsDecl,
};
pub use effects::{
    apply as apply_effects, apply_before, BeforeDecision, BeforeReadings, BeforeRefusal,
    BeforeRequest, EffectCall, EffectEnv, EffectInput, FactDraft, FileOp, ToolBehavior,
};
// 宏用 `$crate::inventory` 展开（卫生），与 `symbio_core` 的 `submit_object_creator!` 同形。
pub use inventory;
pub use rows::{RowHandler, RowRequest, PLUGIN_ROW};
pub use settle::{Contribution, Contributor, SettleFace, Settling};
pub use view_names::SharedViewDecl;
pub use views::{
    read_all_from as read_shared_view_all, read_from as read_shared_view, shared as shared_views,
    ViewRead, ViewUnavailable,
};

// ==================== 扩展层宿主面 ====================
// 本块是**扩展层插件唯一需要认识的宿主入口**：能力包与结构件插件只写
// `symbio_plugin_api::…`，不再直接依赖 `symbio_core` / `symbio-kernel` / `symbio-deploy`。
// 判据由 `plugin-surface-audit`（HS-001）执行：非控制面插件的源码与 manifest 里
// 出现那三个前缀即判红。
//
// **白名单而不是通配**：白名单里每一条都能报出一个真实消费方（与 core 的 C-003 同一把
// 尺子），新增条目要过棘轮（HS-002）。通配（`pub use symbio_core::*`）会让「插件能看见
// 什么」重新变成不可审的隐式集合——那正是本块要消除的东西。
//
// **控制面不在此列**：`loop` / `session` 按 DE-026 的影响类运行，本来就认识宿主的容器
// 概念（`turn` / 幕），继续直接依赖 core；EX-008 的边界正是这一条。把它们也压进本门面，
// 只会逼出一批「端口形状不说容器词」的假豁免。
//
// 装配常量走 `symbio-deploy`（零内部依赖的生成物——插件 id 与装配位是部署契约，
// 不是某一条能力包的私事）。
pub use symbio_deploy::*;

// ---- 语法臂：`symbio-kernel` 的封闭语法 ----
// 这一臂**不计债**：它是内核本来就有的共享类型（事件 / 视图 / 声明容器 / 能力元数据），
// `symbio-kernel` 的直接依赖只有 6 个，插件经它取语法不增加任何机制面。按来源分臂之后，
// 「一条符号属于语法还是机制」在门面上直接可读，HS-002 的棘轮只压机制臂，目标才是可判的
// 0（D-255）。
pub use symbio_kernel::capability::{
    family, Capability, CapabilityCategory, CapabilityFamily, CapabilityMeta, CapabilityRiskLevel,
    CapabilityRow, Registry,
};
pub use symbio_kernel::decl::{ActorDecl, Builder, FactDecl, PluginDeclarations, ViewDecl};
pub use symbio_kernel::dialog::{
    ComposeRequest, DecideOutcome, DecideRequest, DecisionPlan, ParseVerdictRequest, RunSnapshot,
    Verdict, WordingReply, REASON_ACK, REASON_CLARIFY, REASON_EMPTY, REASON_FROM_CONTEXT,
    REASON_GREETING, REASON_NEEDS_WORK, REASON_REFUSE, REASON_THANKS, REASON_UNCLASSIFIED,
};
pub use symbio_kernel::event::{
    EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
    EVENT_TRIGGER_KIND_SCHEDULED, EVENT_TRIGGER_META_KEY,
};
pub use symbio_kernel::exec::{
    ExecAbortSignal, ExecEnv, ExecEventSink, ExecTranscriptWriter, ABORT_SIGNAL, EVENT_SINK,
};
pub use symbio_kernel::keys::SymbioKey;
pub use symbio_kernel::llm::model_provider::{
    ModelFinishReason, ModelProvider, ModelSpend, ModelUsage,
};
pub use symbio_kernel::llm::{PromptMessage, TurnOutput, TurnToolCallInfo};
pub use symbio_kernel::plugin::error::{lock_read, lock_write, PluginError, PluginInvokeResponse};
pub use symbio_kernel::plugin::{
    Plugin, PluginChannel, PluginFrame, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginMessageWire, PluginMeta, PluginPayload, PluginPayloadWire, PluginSimpleRequest,
    PluginStopReason, SimpleResponse, SuccessResponse,
};
pub use symbio_kernel::present::detail::{
    DetailAction, DetailBadge, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailPreset, DetailPresetSpec, DetailSection,
};
pub use symbio_kernel::store::{EventStore, Store};
pub use symbio_kernel::vdfs::{
    vdfs_derive_ext, vdfs_has_parent_segment, vdfs_path_within, DynVdfsProvider, VdfsAccess,
    VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsItem,
    VdfsListResponse, VdfsNewType, VdfsNode, VdfsParams, VdfsProvider, VdfsRequest, VdfsResponse,
    VdfsResult, VdfsValidationError, VdfsWriteResponse, VDFS_ACTION_DISABLE, VDFS_ACTION_ENABLE,
    VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT, VDFS_ACTION_PLUGINS, VDFS_ACTION_TEST, VDFS_EXT_FORM,
    VDFS_EXT_ZIP, VDFS_KIND_DIR, VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT, VDFS_PARAM_WORKDIR,
    VDFS_PLUGINS_FIELD, VDFS_PLUGIN_NAME_FIELD, VDFS_PLUGIN_PROVIDER_FIELD, VDFS_STATUS_ACTIVE,
    VDFS_STATUS_DISABLED, VDFS_STATUS_FAILED, VDFS_STATUS_NONE, VDFS_STATUS_UNKNOWN,
    VDFS_STATUS_WORKING,
};
pub use symbio_kernel::view::Budget;
pub use symbio_kernel::{
    ActorSpec, Entity, Event, EventDecl, FactRole, LatencyTier, Pattern, Scope, Seq, Verb,
};

// ---- 机制臂：`symbio-core` 的宿主机制 ----
// 这一臂是**欠账**：每一条都要 core 的机制支撑（`submit_object_creator` 的链接期收集、
// `PluginDir` 的配置 IO、`capability_invoke` 的调用栈、`vdfs_*` 的地址装配），所以
// `symbio-plugin-api` 至今仍依赖 `symbio-core`。压缩这一臂只有一条路：把它指向的宿主子系统
// 从 core 剥成只依赖 kernel 的层（D-253「不是搬运，是切实现面」）。
// ---- SDK 臂：`symbio-sdk` 的宿主机制（不依赖 core，D-258） ----
// 这一臂的每一条都只碰内核词汇与自己的表——配置 IO、事件总线的订阅表、VDFS 地址装配、
// 对象构造器的链接期登记、日志与协议键。它们与「插件要用一条机制」是同一件事，
// 与「插件要认识整个宿主」被绑成同一件事。SDK 把这条绑定切开：这一臂只依赖内核。
pub use symbio_core::{
    actor_can_assign_work, agent_perms_of, capability_announce_configurable,
    capability_configurables, capability_entry_of, capability_invoke, capability_model_active,
    capability_model_resolve, capability_option_fields, capability_prompts,
    capability_report_error, capability_take_errors, capability_to_wire, capability_vdfs_root,
    chat_node, chat_request, check_all, commitment_events, cost_ledger, failure_kind,
    impl_config_mount_vdfs, llm_emit_message, llm_removed_frame, llm_short_id, node_build,
    plugin_dir_from_ctx, plugin_expand_tilde_path, principal, principal_of, production_matrix,
    readyset_for, ring, transcript, AgentPermissions, AgentScope, ChatVocab, CommitmentVocab,
    DeltaSink, EmbeddingService, FullModel, LlmAdapter, MessageVocab, PluginConfigFile,
    PluginConfigMount, PluginDir, PluginEntry, ProviderLlmAdapter, ReadySetView, SilentDeltas,
    TaskVocab, TokenIssuer, AGENT_PERMS, CAPABILITY_VISITOR, CAP_ASSERT_VERIFICATION,
    CAP_DEFINE_WORK, CONFIGURABLE_VISITOR, EMBEDDING_LOCAL, OPTION_VISITOR, PLUGIN_DIR,
    PLUGIN_FILE, PLUGIN_KEY_CAN_DISABLE, PLUGIN_KEY_ENABLED, PLUGIN_KEY_NAME, PLUGIN_KEY_PROVIDER,
    PLUGIN_KEY_REQUIRED, PLUGIN_KEY_VERSION, PRINCIPAL_MAIN, PRINCIPAL_SCHEDULER, PRINCIPAL_USER,
    TASK_CAP, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
};
pub use symbio_sdk::clock::clock_now_ms;
pub use symbio_sdk::creator::{creator_create_object, creator_has, creator_ids};
pub use symbio_sdk::event_bus::{
    event_bus_build_envelope, event_bus_register_subscriber, event_bus_resync_of,
    event_bus_unregister_subscriber, event_bus_vdfs_change_of, EventBus, EventBusSubscribeRequest,
    EVENT_BUS_KIND_SYSTEM, EVENT_BUS_KIND_VDFS, EVENT_BUS_RESYNC_MARKER_TYPE,
};
pub use symbio_sdk::keys::{
    AGENT_ID, MODE, PATH, PROVIDER_ID, REQUIRED_PLUGINS, RESULT_MSG_ID, RISK_LEVEL, SESSION_ID,
    TOOL_CALL_ID, TRACE_ID, VDFS_PARENT_ADDR, WORKDIR,
};
pub use symbio_sdk::text::{text_floor_char_boundary, text_truncate_bytes};
pub use symbio_sdk::vdfs::{
    absolute_addr, descend_addr, join_addr, vdfs_context, vdfs_from_plugin_error, vdfs_host_ctx,
    vdfs_notify_change, vdfs_unwatch_changes, vdfs_watch_changes, AddrRootDecl,
};
pub use symbio_sdk::{plugin_debug, plugin_error, plugin_info, plugin_warn, submit_object_creator};
