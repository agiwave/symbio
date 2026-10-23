//! 一次轮次的**进程内交付包**：session → loop 的唯一输入面（D-086 §2/§3）。
//!
//! 它只装三类东西：纯值输入（[`TurnSpec`]）、本会话事实轨端口（`Arc<dyn Store>`）、
//! 世界端口（sink / writer / abort / provider / 作用域容器）。这些对象都不跨进程，
//! 故 [`TURN_DISPATCH`] 是 `WIRE = false` 的 typed ctx 键——与 `SESSION_HANDLE`
//! 同一条纪律；它也不是线上协议，`payload` 仍只放 `session_chat::Request`。
//!
//! 结局方向走 [`TurnReport`]（序列化进 `session/<sid>/_run/<op>` 的动作响应），
//! 不在这里开回调面：loop 不接触 `SessionStateChange` / `OUTCOME_*`，那些留守 session。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::symbio_core::chat_node::{ChatNode, NodeStatus};
use crate::symbio_core::event::Event;
use crate::symbio_core::exec::{ExecAbortSignal, ExecEventSink, ExecTranscriptWriter};
use crate::symbio_core::llm::model_provider::ModelProvider;
use crate::symbio_core::node_build::PresentationTree;
use crate::symbio_core::plugin::Plugin;
use crate::symbio_core::store::Store;
use crate::symbio_core::{Scope, SymbioKey};

/// 驱动器能知道的**全部会话输入**（纯值，不含存储实现）。
#[derive(Debug, Clone)]
pub struct TurnSpec {
    /// 地址与幂等键的唯一身份。
    pub session_id: String,
    /// 预计算：`paths::session_subdir(..., TOOL_ARCHIVES_SUBDIR)`。
    pub tool_archive_dir: PathBuf,
    /// 预计算：`paths::session_subdir(..., TRANSCRIPTS_SUBDIR)`。
    pub transcript_archive_dir: PathBuf,
    /// 本会话主体（`request_principal` 的取值点，派发前定）。
    pub principal: String,
    /// 子会话 ⇒ `Child(父主体)`、顶层 ⇒ `Root`；session 从 metadata 预计算。
    pub scope: Scope,
    /// 临时会话判定（今日 `store_handle().is_persistent()`）。
    pub persistent: bool,
    /// 今日 `session.context_window()`。
    pub context_window: u64,
    /// 今日驱动器读到的配置标量。
    pub settings: TurnSettings,
}

/// 驱动器真正读的配置标量（存储策略字段不过墙，属 session 的领域判定）。
///
/// worker 三旋钮的**出厂值**与 session 配置面共用下面三个 core 常量：
/// 判定侧（loop）与配置面（session）不得各持一份字面量。
#[derive(Debug, Clone)]
pub struct TurnSettings {
    pub auto_compress: bool,
    pub classify_enabled: bool,
    pub compose_enabled: bool,
    pub opening_enabled: bool,
    pub progress_enabled: bool,
    pub progress_interval_ms: u64,
    pub progress_min_rounds: usize,
    pub progress_max_per_turn: u32,
    pub presentation_tree: PresentationTree,
    pub skill_compile_enabled: bool,
    pub skill_fast_path: bool,
    pub worker_force_prefix: String,
    pub worker_keywords: String,
    pub worker_min_chars: usize,
}

/// 出厂值：worker 显式前缀。
pub const WORKER_FORCE_PREFIX: &str = "/work ";
/// 出厂值：worker 关键词（子串、忽略大小写）。**空集 = 本层关闭**。
pub const WORKER_KEYWORDS: &[&str] = &[];
/// 出厂值：worker 主题长度阈值（字符数）。**0 = 本层关闭**。
pub const WORKER_MIN_CHARS: usize = 0;

/// 一次轮次的进程内交付包。
///
/// `facts = None` ⇒ 临时会话（今日 `session.facts()` 的同义），driver 沿用
/// 「无事实源 ⇒ 不落派生事实」的分支。
pub struct TurnDispatch {
    pub spec: TurnSpec,
    /// 本会话事实轨端口（P6 已构造好的 `Arc<SessionStore>` 取出的同一实例）。
    pub facts: Option<Arc<dyn Store<Event = Event>>>,
    /// 执行期帧出口（今日 `run_chat_loop` 的入参）。
    pub sink: ExecEventSink,
    /// 压缩增量改道用的转写写口（今日 `CompressionEmitter::transcript_writer`）。
    pub writer: Arc<dyn ExecTranscriptWriter>,
    /// 中止信号（今日 `run_chat_loop` 的入参）。
    pub abort: ExecAbortSignal,
    /// 本轮唯一生效模型服务（既有 core trait）。
    pub provider: Arc<dyn ModelProvider>,
    /// 作用域容器：loop 经它取 VDFS 视图回到 session（不持兄弟插件实例）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本轮生效上下文上限（session 在派发前已解析）。
    pub context_limit: u32,
}

/// loop 交回 session 的唯一结局值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnReport {
    pub status: TurnStatus,
    /// `Failed` 的原因。
    pub error: Option<String>,
    /// `Aborted` 时已产出的半截消息（由 loop 私有的退出类型携带过来——
    /// 它不住 `PluginError`：那是不跨墙的数据，D-095）。
    pub partial: Vec<ChatNode>,
    /// Stop 钩子的 `last_message`（今日 `fire_stop_hook` 的取值）。
    pub last_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Aborted,
    Failed,
}

/// 进程内交付键：只在本进程的 ctx 上出现，线上不可伪造。
pub struct TurnDispatchKey;
impl SymbioKey for TurnDispatchKey {
    type Value = Arc<TurnDispatch>;
    /// 进程内专用：trait object / sink 无字符串 / JSON 形态。
    const WIRE: bool = false;
    fn name(&self) -> &'static str {
        "turn_dispatch"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "turn_dispatch".to_string()
    }
}
pub const TURN_DISPATCH: TurnDispatchKey = TurnDispatchKey;

/// loop→session 内部动作段的**地址词表**（[D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §4；
/// 闭集的去处见 [D-094](../../../../docs/003-design/014-单环一表.md#d-094-内部动作段注册化行表即闭集run_opsall-删除sessionport-收成一个-call "DE-014-单环一表")）。
///
/// `_run` 是会话挂载点下的一级段：`session/<sid>/_run/<op>`。地址末段与
/// `Action.action` 必须**字面相等**——两者只在这里命名一次，两侧都从这里取。
///
/// 为什么词表住 core：地址的拼方（loop）与解方（session）隔着插件墙（E-009），
/// 而「有哪些 op」是两侧共同的契约，任一侧私有都会长出第二份字符串常量。
/// 它不是资源段：不出现在 `list` 里（`internal_dirs` 不收它）。
///
/// **闭集不在这里**：「session 实现了哪些动作」是执行侧事实，与处理函数同住
/// session 的行表（D-094）——授权判据第 3 条读那张表，「在表内」与「有臂」
/// 是同一件事。清单与实现分居两地要靠人工对账，而漏一份是静默的授权缺口。
pub mod run_ops {
    /// 内部动作段的段名。
    pub const SEG: &str = "_run";

    pub const MESSAGES: &str = "messages";
    pub const CONTEXT_MESSAGES: &str = "context-messages";
    pub const ADD_BY_RUNNER: &str = "add-by-runner";
    pub const ADD_HERE: &str = "add-here";
    pub const EDIT_NODES: &str = "edit-nodes";
    pub const REMOVE_NODES: &str = "remove-nodes";
    pub const REPLACE_ALL: &str = "replace-all";
    pub const DRAIN_PARKED: &str = "drain-parked";
    pub const CONSOLIDATE: &str = "consolidate";
    pub const TAKE_SUPPLEMENTS: &str = "take-supplements";
    pub const WORKER_PROGRESS: &str = "worker-progress";
    pub const RECALL_SNAPSHOTS: &str = "recall-snapshots";
    pub const ELIMINATION_REVOKED: &str = "elimination-revoked";
    pub const COMPRESSION_BEGIN: &str = "compression-begin";
    pub const COMPRESSION_FINISH: &str = "compression-finish";
}

/// session → loop 的唯一动作名（[D-086] §5）：`loop/<sid>` 只接受它。
///
/// 两侧隔着插件墙各写一遍字面量会静默漂移（拼方与解方必须逐字相等），
/// 故与 `run_ops` 同住 core——它是**纯值词汇**，不是新机制。
///
/// 决策 D-086 见 [D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")
pub const LOOP_RUN_TURN: &str = "run_turn";

// ── 动作载荷（纯值，两侧共同命名）──────────────────────────────────────
//
// 动作的「请求体 / 回体」与 op 词表同住 core：拼方（loop）与解方（session）
// 隔着插件墙，任一侧私自定义都会长出第二份字段名。它们只走进程内 VDFS 动作的
// `payload` / `data`，**不是**对外 API。

/// `add-by-runner` / `add-here` 的请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddNodesRequest {
    pub messages: Vec<ChatNode>,
}

/// `edit-nodes` 的帧形态：状态帧只带状态迁移，完整消息帧带整条正文。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditFrame {
    State,
    Message,
}

/// `edit-nodes` 的请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditNodesRequest {
    pub nodes: Vec<ChatNode>,
    pub frame: EditFrame,
}

/// `remove-nodes` 的请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveNodesRequest {
    pub ids: Vec<String>,
}

/// `replace-all` 的请求体（L2 语义压缩：整表重写 + 旧首段删除 + 新首条）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceAllRequest {
    pub nodes: Vec<ChatNode>,
    pub dropped: Vec<String>,
    pub head: ChatNode,
}

/// `consolidate` 的请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidateRequest {
    pub messages: Vec<ChatNode>,
}

/// `compression-begin` 的请求体：`force` 绕过熔断（用户主动重试）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionBeginRequest {
    pub node_id: String,
    #[serde(default)]
    pub force: bool,
}

/// `compression-begin` 的回体：`skipped = true` ⇒ 熔断中，未插入节点。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompressionBeginReply {
    pub skipped: bool,
}

/// `compression-finish` 的请求体：定稿节点 + 熔断记账同一次动作。
///
/// 记账随定稿走是刻意的：熔断计数是**会话级**节奏状态，只有 session 知道；
/// loop 只报结局（[`CompressionOutcome`]），不自己留着第二份计数器。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionFinishRequest {
    pub node_id: String,
    pub status: NodeStatus,
    pub text: String,
    #[serde(default)]
    pub failure_kind: Option<String>,
    #[serde(default)]
    pub stats: Option<serde_json::Value>,
    #[serde(default)]
    pub outcome: CompressionOutcome,
}

/// 一次压缩请求的结局（供 session 更新熔断计数）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionOutcome {
    /// 没真正跑内核（无切分点 / 无收益 / 用户重试未触发）：不记。
    #[default]
    None,
    Success,
    Failure,
    /// 输入超限类：只有人工处理（换更大上下文的模型）才会恢复。
    PermanentFailure,
}
