//! 核心模块

// Rule-1（架构出口唯一）：core 的代码一律住子目录，但子目录**不对外开模块门**——根一律
// `mod <域>;` 私有，谁出现在公共面上由下方 `pub use` 逐条显式声明（`scripts/core-export-audit.mjs`
// C-001 校验，编译器兜底）。外部只写 `symbio_core::<符号>`，不得写 `symbio_core::<域>::…`。
mod actors;
mod adapters;
mod assembly;
// **本机授权表**（谁持有什么能力 + 可见域）——与下面 `governance` 的机制同住一棵模块树：
// 表与判定分家的唯一坏处是「表授予 A、闸门判 B」这种错位**没有任何东西会变红**。
// 表整块是架构元素，故保留 `authz` 命名空间、`pub(crate)` 开给全 crate（不进对外公开面，
// 故 C-003 不数它）：表里写的是部署事实，不是契约。
pub(crate) mod authz;
mod capability;
mod clock;
mod creator;
mod embedding;
mod event;
mod event_bus;
mod exec;
mod governance;
mod invariants;
mod keys;
mod llm;
mod logger;
mod plugin;
mod projection;
mod schemas;
mod store;
mod text;
mod vdfs;
mod view;

// ==================== 事件溯源契约（v2 阶段 S0） ====================
// 共享类型层（`event` / `view`）与两个机制（`store` ④ / `projection` ③）。
// 模块依赖纪律见 [plan/05 §3.1]：彼此只准依赖类型定义，不持有对方句柄。
pub use event::{Entity, Event, EventEnvelope, Seq, Verb, EVENT_TURN_SUPPLEMENTED};
pub use event::{
    EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_REPORTED, EVENT_USER_MESSAGE,
};
pub use invariants::check_all;
pub use projection::calibration::calibration;
pub use projection::checkpoint::checkpoint;
pub use projection::cost::cost_ledger;
pub use projection::fallback::fallback_rate;
pub use projection::slo::slo_report;
pub use projection::transcript::transcript;
// 结构化 prompt 消息（ADR-048a）：core 的**最小结构化出口**——角色、正文、
// `tool_call_id`、`tool_calls`。线格式（`ChatMessage`）由 adapter 边界翻译，
// core 内部不认识 provider 的图语义。
//
// **为什么它占根出口而 `PromptToolCall` 不占**：C-003 的准则是「只有一个模块消费
// 的不算架构元素」。`PromptMessage` 跨了 core 边界——`LlmAdapter` 是 core 的公开
// trait，它的入参类型不能是 core 私有的（否则 core 外**根本无法实现那个 trait**，
// 测试替身 / 外部适配器都写不出来）。`PromptToolCall` 只被 `actors` 用，留在模块路径。
pub use projection::transcript::PromptMessage;
pub use projection::turnstate::{turnstate, TurnState};
pub use store::wal::{EventWalStore, WalStore};
pub use store::{EventStore, Store};
pub use view::Budget;

// ==================== 主体（v2 阶段 S1，② actors） ====================
// 只收类型化输入、只产事件（plan/05 §3.1 ② 行）；S1 落 Decider 平凡值，S2 加 Reasoner。
//
// ⚠️ `TurnRunner` / `TurnInput` / `TurnOutcome` / `TurnResume` / `RoundInjector`
// **已不在本层**（2026-10-09 下沉到 `plugins/session/turn_runner.rs`）：它们是**执行引擎**
// （流循环 + 落格顺序），生产消费方只有 session 一个，按 README §4 四问第 1 问该下沉；
// 且它们**不是** F1–F6 冻结锚点（F3 冻的是 `ActorSpec` 字段集 + 档位→令牌映射，不是运行器）。
// 曾登记为 `plan/01 §4` 冻结契约名是**误记**——`plan/01` 全文没有 `TurnRunner` 一词。
//
// `ActorSpec`（F3 冻结锚点：`plan/01 §4` 的**主体规格五字段**）**留根出口**（`pub use`，
// 不是 `pub(crate)`）。下沉前它靠 `TurnInput.actor`（当时的根导出）**顺带**可达，故
// `pub(crate)` 也够用；运行器搬走后这条顺带路径没了，而 F3 冻的是**字段集**、它只能住
// core ⇒ 必须显式占根出口，否则 core 外（`plugins/session` 构造 / 读 `principal`）够不着它
// （C-002 禁止插件深引域目录）。它只有 1 个生产消费方（`plugins/session`），属 C-003
// 的「接缝」而非「该下沉」——下沉即等于把 F3 冻结契约搬出 core，故登记进
// `core-export-audit.mjs` 的 `WAIVERS`（与 `PermissionMatrix` / `VisScope` 同一形态）。
pub use actors::{ActorSpec, Pattern, Reasoner};
// 外部执行闸门（S8 第 20 步）、插话抢占判定（S8 第 19 步）、自主发起 + 意图闸门
// （S9 第 21 步）与技能编译 + 路由（S9 第 22 步）：判定在 core、接线在
// `plugins/session`（闸门进工具执行闸，判定者进收件箱忙窗，自主侧进心跳 tick，
// 技能进 `v2_skills`）。**必须整对**：闸门与自主层同批成对（否则"自主写入对话"，
// 04 §3.1），编译与路由同批成对（否则回退永远不会发生——编译不入格就没有
// `skill_id`、路由不落观测就没有 `fallback`，置信度恒 1.0，S11 §5 静默失效）。
// `pub(crate) use` 而非 `pub use`：消费方全在本 crate 内，进根公开面只会给
// C-003（根导出须 ≥2 个消费方）凭空添待办——C-003 只数 `pub use`。
pub(crate) use actors::{
    AutonomousInitiator, CircuitBreaker, ConationCandidate, ConationPolicy, GateDecision,
    IntentGate, Preemption, PreemptionDecider, SkillCompiler, SkillRoute, SkillRouter,
};
// 事件名字表（名字是数据，单点定义）。
// 任务格名字表（S7 步 16–18，批⑨）：core 内的读方（`projection::readyset`、
// `invariants` 的 `acyclic_deps` / `rework_bounded`）走域内路径，core 外的写方只有
// `plugins/session/v2_tasks` 按名字落格——**单消费方**，故与记忆事件名字表（下）、
// `readyset` 出根（同批）同一形态：`pub(crate)` 出根（C-003 数 `pub use`），只在本
// crate 内可见、不进对外公开面。收窄成「下沉到 `plugins/session`」会让写方复述字面量、
// 制造第二处真相，违反 ADR-043「名字是数据、单点定义」——可下沉的只有消费方，字典不行。
pub(crate) use event::{
    EVENT_TASK_ASSERTED, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED,
};

// ==================== 记忆三段（v2 阶段 S5 步 11–13） ====================
// 写方 / 召回注入 / 巩固的消费方只有 `plugins/session` 的记忆链路**一个**。C-003 的
// 口径是「单消费方不算架构元素，该下沉」——可下沉的只有消费方，算法不行：
// `recall` 是 ③ 的投影、`consolidate` 是 01 §9.3 的接受边界、事件名字表是 ADR-043
// 的单点定义，都必须留在 core。故按 `symbio_core/README.md §4 四问` 留 core，以
// **`pub(crate)`** 出根：只在本 crate 内可见、不进对外公开面（C-003 数 `pub use`），
// 与 `lock_read` / `ObjectConstructor` 同一形态。记忆事件名字表同样只对写方可见——
// 名字是数据，core 内与插件内各写一份字面量就等于两套事件名。
// `recalled_event` 出根而不是主体类型本身：NDC-001（无直连）禁止定义域之外**提及
// 主体名**——提及即可持有、持有即可绕过事实源。主体仍在 `actors` 内被本函数驱动。
pub(crate) use actors::{commitment_events, recalled_event};
// prompt 的结构化渲染出口（`Reasoner::render_messages` 的一跳封装）：运行器下沉
// `plugins/session` 后要「事件切片 → 消息数组」，但 NDC-001 禁止它在定义域之外提及主体名
// （`scripts/no-direct-call-audit.mjs`）⇒ 与 `recalled_event` 同一形态，主体仍在 `actors`
// 内被本函数驱动。生产消费方是 session 的**生产**代码，故不带 `cfg(test)`。
pub(crate) use actors::render_messages;
// 承诺事件名字表：core 内的消费方（`commitment_events` 的 `CommitmentKeeper`、
// `projection::reputation`）走**模块内**路径引用，插件侧只有**测试**要按名字断言
// （`v2_bridge.test.rs`）——而 C-002 禁止 core 外深引、非测试构建里这四个名字又
// 没有消费者。故只在 `cfg(test)` 出根：出根是给测试用的，不出根不是把它们藏着。
#[cfg(test)]
pub(crate) use event::{
    EVENT_COMMITMENT_ASSERTED, EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OFFERED,
    EVENT_COMMITMENT_RELEASED,
};
pub(crate) use event::{
    EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN, EVENT_MEMORY_RECALLED,
};
pub(crate) use projection::consolidate::{accept, ConsolidateParams, Rejection};
// 就绪集出根：生产消费方是 `plugins/session` 的**两处**——读出口 `session/stats`
// 的 `readyset` 列与请求装配的调度段（04 §3.1 批⑨）。与 `reputation` 同一形态：
// `pub(crate)` 出根，只在本 crate 内可见、不进对外公开面（C-003 数 `pub use`）。
pub(crate) use projection::conation::conation;
pub(crate) use projection::readyset::readyset;
pub(crate) use projection::recall::recall;
pub(crate) use projection::reputation::reputation;
pub(crate) use view::RecallView;
// 可见域判据与智能体身份前缀：三处共用**同一条**判定——写视图的 `window_by_turn`
// （core）、写消息过滤的 `context/view`（插件）、做主体派生的授权表（`authz`，本根开给全 crate）。
// 两个判定方 = 两套可见域与两种身份形状，迟早漂移成「判的是 A、写的是 B」。
pub(crate) use view::{visible_to, AGENT_PREFIX};

// ==================== 适配器（v2 阶段 S2，⑤ adapters） ====================
// 时延闸门 = 本包的依赖注入策略（plan/05 §3.3）：令牌按档位签发，反射档拿不到模型句柄。
// 两只端口并列：`LlmAdapter`（生成）与 `DispatchPort`（工具分发）——运行器只认这两只，
// 不认任何具体实现（插件宿主/转写都在实现侧）。
pub use adapters::{
    DeltaSink, DispatchOutcome, DispatchPort, LatencyTier, LlmAdapter, LlmTurn, ProviderLlmAdapter,
    TokenIssuer,
};
// 执行引擎（`TurnRunner` 族）2026-10-09 下沉 `plugins/session/turn_runner.rs` 后，仍需要的
// core 内部面。它们**不能下沉**——core 自己也在用（`TokenIssuer` 铸令牌、`provider_adapter`
// 实现 trait、`store` 读写、`invariants` 判据、`prompt_fidelity` 复核）；插件又不能深引域
// 目录（C-002）⇒ 以 `pub(crate)` 出根：只在本 crate 内可见、不进对外公开面（C-003 只数
// `pub use`），与 `visible_to` / `recall` 同一形态。（`ActorSpec` 不在此列——它是 F3 冻结
// 锚点，另以根 `pub use` 出，见上。）
pub(crate) use adapters::{AdapterError, FullModel, RuleOnly};
// 空增量口 `SilentDeltas` 的**根出口**只剩测试在用：生产侧 `v2_exec` 自带 `SilentDeltaSink`
// （见其注释），core 内 `Reasoner::reply_timed` 走 `adapters::` 域内路径 ⇒ 同 `StubLlmAdapter`
// 一形态，只在 `cfg(test)` 出根（下沉后的 `turn_runner.test.rs` 是唯一根出口消费者）。
#[cfg(test)]
pub(crate) use adapters::SilentDeltas;
// 零 LLM 桩只在测试构建里存在（README §1.2 adapters 行）——`cfg(test)` 出根给下沉后的测试用。
#[cfg(test)]
pub(crate) use adapters::StubLlmAdapter;
// C4 判据（`unresolved_turns`）只在**测试**里经根出口用：生产侧没有任何调用点（`check_all`
// 在 core 内走域内路径），下沉后的 `turn_runner.test.rs` 是唯一根出口消费者，而 C-002 禁止
// 它深引 `symbio_core::invariants::…` ⇒ 同 `StubLlmAdapter` 一形态，只在 `cfg(test)` 出根。
#[cfg(test)]
pub(crate) use invariants::unresolved_turns;
// 事实网格 → prompt 的双向完整性复核（ADR-048 防线节）：下沉后的运行器组装 prompt 时调它。
pub(crate) use projection::prompt_fidelity::verify as verify_prompt_fidelity;
// `PromptMessage` 的结构化子件：`PromptMessage.tool_calls` 的载荷类型。此前只被 core 内
// `actors` 用（故根注释曾写「留在模块路径」）；运行器下沉 `plugins/session` 后它跨出 core，
// 而 C-002 禁止插件深引域目录 ⇒ 随 `PromptMessage` 一起以 `pub(crate)` 出根。
pub(crate) use projection::transcript::PromptToolCall;
pub(crate) use store::AppendError;

// ==================== 权限与可见性（v2 阶段 S3，⑥ governance） ====================
// 读写成对、fail-closed（plan/01 §7）。根出口只出**矩阵与可见域**两个类型：
// 构造面（本根 `authz` 的主体清单）与判定面（`plugins/session` 两道闸）都要指称它们。
// core 外消费方只有 `plugins/session` 一个，但它们**不能**下沉——core 内 `authz` /
// `governance` 自己也在用（core 不依赖插件），插件又够不着 `governance` 域目录的深引
// （C-002）⇒ 必须留根出口。故按 C-003 的接缝口径登记进 `core-export-audit` 的 WAIVERS。
// governance::Capability（权限能力，7 个封顶）与 capability::Capability（LLM 工具
// 描述符）是两个概念，**刻意不进根平铺**：授权表走能力名字符串（由
// `PermissionMatrix::from_names` 按闭集校验），轮次→能力的映射收在 `can_reply` 里，
// core 外的调用点既不深引也不与 `Capability` 撞名，见 governance/mod.rs 同名辨析。
pub use governance::{PermissionMatrix, VisScope};

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
    VDFS_ACTION_TRUNCATE, VDFS_EXT_FORM, VDFS_EXT_ZIP, VDFS_KIND_DIR, VDFS_PARAM_BEFORE,
    VDFS_PARAM_LIMIT, VDFS_PARAM_WORKDIR, VDFS_PLUGINS_FIELD, VDFS_PLUGIN_NAME_FIELD,
    VDFS_PLUGIN_PROVIDER_FIELD, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED, VDFS_STATUS_FAILED,
    VDFS_STATUS_NONE, VDFS_STATUS_UNKNOWN, VDFS_STATUS_WORKING,
};
// 域类型：节点 / 内容 / 请求响应 / 错误 / 变更
pub use vdfs::{
    DynVdfsProvider, VdfsAccess, VdfsActionResult, VdfsChange, VdfsChangeSink, VdfsContent,
    VdfsContext, VdfsError, VdfsItem, VdfsNewType, VdfsNode, VdfsParams, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsValidationError, VdfsWriteResponse,
};
// 契约函数与宿主桥
pub use vdfs::{
    vdfs_change_of, vdfs_context, vdfs_derive_ext, vdfs_from_plugin_error, vdfs_has_parent_segment,
    vdfs_host_ctx, vdfs_notify_change, vdfs_path_within, vdfs_unwatch_changes, vdfs_watch_changes,
    VdfsChangeSubscriptions,
};
// 地址机制（crate 内部装配用，不是公开契约）
pub(crate) use vdfs::{absolute_addr, descend_addr, join_addr, AddrRootDecl};

// ==================== 跨插件契约（schemas，ADR-023） ====================
// `schemas/` 目录同样不开模块门：schema 契约在根显式再导出——`chat_message` /
// `session_chat` 保**叶子模块**形态（`as cm` 别名依赖它），其余按符号扁平导出。
// 外部一律 `symbio_core::ChatMessage` / `symbio_core::chat_message::…`。
pub use schemas::common::{SimpleResponse, SuccessResponse};
pub use schemas::detail::{
    DetailAction, DetailBadge, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailPreset, DetailPresetSpec, DetailSection, DETAIL_PICK_DIRECTORY,
};
pub use schemas::session::{chat_message, session_chat};
pub use schemas::{
    ComposeRequest, DecideRequest, HookEvent, HookOutput, RunSnapshot, Verdict, REASON_ACK,
    REASON_CLARIFY, REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING, REASON_NEEDS_WORK,
    REASON_REFUSE, REASON_THANKS, REASON_UNCLASSIFIED,
};

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
    capability_to_wire, Capability, CapabilityCategory, CapabilityMeta, CapabilityRiskLevel,
    CapabilityToolContextRetention, CapabilityVisitor, ConfigurableVisitor, OptionVisitor,
};
// 工具结果 `failure_kind` 闭集：生产方（`local`）与消费方（`session`）分属不同插件，
// 互相不可见，只能经这里共享。单独一行——它是模块而非类型。
pub use capability::failure_kind;
pub use plugin::{
    plugin_dir_from_ctx, plugin_expand_tilde_path, Plugin, PluginChannel, PluginConfigFile,
    PluginConfigMount, PluginDir, PluginEntry, PluginError, PluginFrame, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMessageWire, PluginMeta, PluginPayload,
    PluginPayloadWire, PluginSimpleRequest, PluginStopReason, PLUGIN_FILE, PLUGIN_ID_AGENT,
    PLUGIN_ID_CLASSIFY, PLUGIN_ID_COMPOSE, PLUGIN_ID_COMPOSITE, PLUGIN_ID_EVENT_BUS,
    PLUGIN_ID_GATEWAY, PLUGIN_ID_HOME, PLUGIN_ID_HOOK, PLUGIN_ID_LOCAL, PLUGIN_ID_MANAGER,
    PLUGIN_ID_MCP, PLUGIN_ID_MEMORY, PLUGIN_ID_MODEL, PLUGIN_ID_SESSION, PLUGIN_ID_SETTING,
    PLUGIN_ID_SKILL, PLUGIN_ID_TELEGRAM, PLUGIN_ID_VDFS, PLUGIN_ID_WEB, PLUGIN_KEY_CAN_DISABLE,
    PLUGIN_KEY_ENABLED, PLUGIN_KEY_NAME, PLUGIN_KEY_PROVIDER, PLUGIN_KEY_REQUIRED,
    PLUGIN_KEY_VERSION, PLUGIN_PAYLOAD_KEY, ROUTE_CLASSIFY_DECIDE, ROUTE_COMPOSE_WORDING,
    ROUTE_EVENT_BUS_SUBSCRIBE, ROUTE_HOOK_FIRE, ROUTE_SESSION_CHAT_SEND, ROUTE_VDFS_ROOT,
    ROUTE_VDFS_UNWATCH, ROUTE_VDFS_WATCH, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
};
// 注意：submit_object_creator! 宏已通过 #[macro_export] 导出到 crate 根目录

// ==================== 通用对象创建注册表 ====================
// 按 id 装配任意类型对象——**不是插件专属**：注册表按 `(id, TypeId)` 索引，
// 当前服务 `dyn Plugin` / `dyn ModelProtocol` / `dyn EmbeddingService` 三个类型族。
// 独立成域的理由见 `creator/mod.rs` 的模块文档。
pub use creator::{creator_create_object, creator_has, creator_ids};
// 注册表内部结构，供 `submit_object_creator!` 宏展开取用（crate 内可见）
pub(crate) use creator::{ObjectConstructor, Submit};

// ==================== 键面 ====================
pub use keys::{
    SymbioKey, ABORT_SIGNAL, AGENT_ID, CAPABILITY_ERRORS, CAPABILITY_VISITOR, CONFIGURABLE_VISITOR,
    CONTENT, EVENT_SINK, ID, MODE, NAME, OPTION_VISITOR, PARENT, PATH, PLUGIN_DIR, PROVIDER_ID,
    REQUIRED_PLUGINS, RESULT_MSG_ID, RISK_LEVEL, SESSION_ID, TOOL_CALL_ID, TRACE_ID,
    VDFS_PARENT_ADDR, WORKDIR,
};

// ==================== 装配策略 ====================
// 「一棵标准插件树挂哪些插件」「哪些插件不许被停用」——不是键、不是 id，见 assembly 模块文档。
pub use assembly::{ASSEMBLY_SUB_AGENT_PLUGINS, ASSEMBLY_UNDISABLABLE_PLUGINS};

// ==================== 基础设施 / 工具函数 ====================
// 注：`CAPABILITY_ERRORS`（错误桶键）不在这里——它是 `SymbioKey` 实例，
// 随 `pub use keys::*` 一并平铺（键面只有一个定义处，见 `keys/mod.rs`）。
pub use capability::{
    capability_init_error_bucket, capability_report_error, capability_take_errors,
};
pub use clock::clock_now_ms;
// 锁辅助函数**刻意不进 `pub use plugin::*`**（见 `plugin/error.rs::lock_read` 的说明）：
// 显式 `pub(crate)` 导入，既让全 crate 可用，又保留 `dead_code` 的可见性。
pub use exec::{ExecAbortSignal, ExecEnv, ExecEventSink, ExecTranscriptWriter};
// 注：homedir（系统根注册表）**不在 core**——它归 `home` 插件独有。core 只提供
// 纯路径工具 `plugin_expand_tilde_path`（经 `plugin::dir` 重导出，不读任何全局系统根）。
// 日志门面：`logger_is_initialized` / `logger_level_enabled` / `LOG_LEVEL_{WARN,ERROR}` 只被
// logger 的 `plugin_*!` 宏体引用——但宏在**插件**调用点展开，根路径必须可达（`core-export-audit`
// 的消费方计数认这层，见 `core-surface.mjs` 的 `collectMacroBodies`）。
// `logger_min_level` 只在 core 内用，故不进根。
pub use logger::{
    logger_init, logger_is_initialized, logger_level_enabled, logger_parse_level,
    logger_set_min_level, LOG_LEVEL_DEBUG, LOG_LEVEL_ERROR, LOG_LEVEL_INFO, LOG_LEVEL_WARN,
};
// 注：记忆（`MemoryFile` / 两道闸门 / 片段渲染 / 节点形状）**整块不在 core**——
// 实现不隶属任何单个插件（memory / session 各用一个或多个作用域），故归
// `providers/memory`（方式 B，不套 `dyn`）。**连文件名也不在 core**：各作用域
// 定义自己的文件名常量（`memory::memory::MEMORY_FILE` 等），core 不设统一约定。
pub(crate) use plugin::{lock_read, lock_write};
pub use text::{text_floor_char_boundary, text_truncate_bytes};

// 重导出 inventory 供 submit_object_creator! 宏使用
pub use inventory;
