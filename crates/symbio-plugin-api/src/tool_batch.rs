//! 工具**批形状**合同（M3，D-298 的补件面）。
//!
//! ## 为什么单笔合同不够
//!
//! [`crate::port::ToolCallExecutor`] 只覆盖「一次调用」的三样 glue（线上名解析 /
//! 空闲超时 / 结果契约）。批级的三件事没有合同面可去：批首读数（熔断状态 / 同参
//! 台账）、并发策略（风险档 → 并行 / 串行）、结果守卫（体积上限与归档滚动）。
//! 它们消费的是工具与模型世界的词汇，不是 `(port,id,attempt)` 那种端口无关记账，
//! 因此不能按「环的编排」留在环里——环只保留选债与计数骨架（D-298 的 M3 修订）。
//!
//! ## 边界：合同只搬「读数」与「出参」
//!
//! 输入侧全是**按值读好的读数**与回调：批调用列表、批首闸门读数、风险表、结果
//! 守卫、事件出口。实现侧因此不需要回读事实源、不持有轮容器、不认识声明表——
//! 它只按读数执行，把结果节点与派生事实交回。
//!
//! 输出侧是**出参而不是事件**：节点（供会话镜像落库）与效果 / 熔断理由（供轮末
//! 落格）一起交回，调用方不靠猜。
//!
//! ## 不接管的东西（同一条决策的否定面，写在这里免得被后续「顺手」加进来）
//!
//! - **选债**：`authorize` / `dispatch` / `await-{port}` 行仍在环里；本模块只被
//!   `dispatch` 那一行动作调用。
//! - **授权判据**：风险 vs 阈值、主体 vs 矩阵的比较仍在授权侧。
//! - **重试**：租约 / `attempt` 仍是环的 `port.rs` 的事。
//! - **压缩链**：守卫只做单条结果的体积裁剪；tokenizer 与三层压缩不在本模块。

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use symbio_kernel::capability::{CapabilityMeta, CapabilityRiskLevel};
use symbio_kernel::event::Event;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::llm::TurnToolCallInfo;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest};
use symbio_sdk::chat::chat_node::ChatNode;

use crate::effects::{BeforeReadings, EffectInput};

// ── 批首闸门读数（熔断状态 / 同参台账）────────────────────────────

/// 外部执行闸门的**批首读数**（D-044：实测与判据同源，读台账不读运行时计数器）。
///
/// 判据在批首读一次、整批复用：授权表与成本台账都是只读的，闸门不该为了拿到
/// 自己的判据就把事实源重放进工具循环（几十个调用点 × 全量重放 = 闸门自己变成
/// 时延源）。
#[derive(Debug, Clone, Copy, Default)]
pub struct BreakerInputs {
    /// 主体是否持外部执行能力（读路径判定）。
    pub authorized: bool,
    /// 已耗（会话累计，成本台账）。
    pub spent_ms: u64,
    /// 本次申请（工具循环所在档的额度）。
    pub requested_ms: u64,
    /// 总预算（会话累计成本天花板）。
    pub budget_ms: u64,
}

/// 外部执行闸的一次结论（DE-039 §6）。
///
/// 出口**不对称是合同的一部分**，不是实现细节：`Refuse` 不得产生任何事件，
/// `Break` 必须入格；两者混成一种「拒绝」会让验收 1 / 2 在事件面上分不开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// 未授权——调用方不得产生任何事件（验收 1）。
    Refuse,
    /// 熔断（预算耗尽 / 判定超时）——调用方必须写受控事件（验收 2）。
    Break { reason: &'static str },
    /// 放行执行。
    Allow,
}

/// 批首一次读齐的闸门读数（逐条执行与未执行收口只按值消费，不再回读事实源）。
pub struct BatchGateReadings {
    /// 会话运行模式（`interactive` 下审批 / 提问一次只处理一张卡，整批串行）。
    pub mode: String,
    /// 熔断判据的起算时刻：量「这份输入已经读了多久」。
    pub gate_started: std::time::Instant,
    /// 熔断状态（授权 / 已耗 / 申请 / 预算四格）。
    pub breaker: BreakerInputs,
    /// 工具自声明的行为位（声明闸与效果端口选实现的依据）。
    pub decls: BehaviorDecls,
    /// 前置判断的控制面读数：声明闸在批首拿到同一份结论。
    pub before: BeforeReadings,
    /// 本条批的主体：声明闸都按它问。
    pub principal: String,
    /// 批首的会话事实切片——声明闸读历史用（持到批尾的代价是一份事件值）。
    pub session_facts: Vec<Event>,
    /// 工具自声明的风险档：批内执行策略的分流依据。
    pub risks: ToolRisks,
    /// 同批在飞上限（端口声明；`0` / `1` = 串行）。
    pub concurrency: u16,
    /// 本批留底 `(id, name, args)`——未执行收口的依据。
    pub batch_meta: Vec<(String, String, Value)>,
    /// 同参重复调用台账（逐条执行时登记，故为可变读数）。
    pub seen_calls: Vec<String>,
}

// ── 并发策略（风险档 → 并行 / 串行）──────────────────────────────

/// 一个调用在批内的执行策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecPolicy {
    /// 可与其它 `Parallel` 调用同时在途（结果仍按调用顺序收口）。
    Parallel,
    /// 必须独占执行窗口：前序在途调用先全部收口。
    Serial,
}

/// 风险档 → 执行策略的**唯一映射**（新增一档只改这里）。
///
/// 只把 `Low`（只读、不触碰资源）放进并行；`Medium` / `High` 一律串行——
/// 写文件、改配置这类调用并行会引入顺序不确定。
pub const fn policy_for_risk(risk: CapabilityRiskLevel) -> ExecPolicy {
    match risk {
        CapabilityRiskLevel::Low => ExecPolicy::Parallel,
        CapabilityRiskLevel::Medium | CapabilityRiskLevel::High => ExecPolicy::Serial,
    }
}

/// 会话运行模式 → 整批策略：交互模式整批串行。
pub fn policy_for_mode(mode: &str, risk_policy: ExecPolicy) -> ExecPolicy {
    if mode == "interactive" {
        ExecPolicy::Serial
    } else {
        risk_policy
    }
}

/// 端口声明 → 整批策略：`concurrency <= 1` 是**硬闸**（即使只读、即使 `auto`）。
///
/// 声明与策略是两个问题：端口说「同批最多几笔在飞」，风险档说「哪几条能同时在途」；
/// 前者只能收紧后者，不能替代它。
pub fn policy_for_port(concurrency: u16, mode: &str, risk_policy: ExecPolicy) -> ExecPolicy {
    if concurrency <= 1 {
        ExecPolicy::Serial
    } else {
        policy_for_mode(mode, risk_policy)
    }
}

/// 批首读一次的工具风险表（键 = 线上名与能力名，两者都收）。
///
/// 键收两种形态，因为两条链路各用各的：注册表里是能力名，模型发来的是线上名，
/// 而解析规则是「字面名优先、投影名次之」——本表按同一规则收键（先收投影名，
/// 再用字面名覆盖），于是查表结果与执行期的解析结果同源。
#[derive(Debug, Default)]
pub struct ToolRisks(HashMap<String, CapabilityRiskLevel>);

impl ToolRisks {
    /// 按值复用同一份元数据：执行策略与声明位是两个消费者，批首只遍历一次收集管线。
    pub fn of_metas(metas: &[CapabilityMeta]) -> Self {
        let mut map = HashMap::new();
        // 投影名先收：两个字面名投到同一个线上名时，后收的会覆盖前一个——
        // 与解析器的「字面相等优先」不冲突（那里也先认字面名）。
        for meta in metas {
            map.insert(
                symbio_sdk::capability::capability_to_wire(&meta.name),
                meta.effective_risk(),
            );
        }
        // 字面名后收，覆盖同键的投影值。
        for meta in metas {
            map.insert(meta.name.clone(), meta.effective_risk());
        }
        Self(map)
    }

    /// 线上名 → 生效风险档；查不到 = 未声明 = `Medium`（与 `effective_risk` 同一兜底）。
    pub fn effective_risk(&self, wire_name: &str) -> CapabilityRiskLevel {
        self.0
            .get(wire_name.trim())
            .copied()
            .unwrap_or(CapabilityRiskLevel::Medium)
    }

    /// 本条调用的风险档策略（交互模式另由 [`policy_for_mode`] 压成串行）。
    pub fn policy_for_call(&self, wire_name: &str) -> ExecPolicy {
        policy_for_risk(self.effective_risk(wire_name))
    }
}

// ── 工具自声明的行为名（声明闸与效果端口选实现的依据）────────────────

/// 线上全名 → 声明的行为名（D-210）。
///
/// 匹配口径先**等值**、再**取末段**：注册表的键是能力短名，而线上名带挂载点前缀；
/// 只判等值会让「挂了前缀的工具」在端口里静默没有效果（不是报错，是少记账）。
#[derive(Debug, Default)]
pub struct BehaviorDecls {
    declared: HashMap<String, &'static [&'static str]>,
}

impl BehaviorDecls {
    /// 按值复用同一份元数据；没有 visitor 时元数据为空 ⇒ 声明为空 ⇒ 没有闸命中。
    pub fn of_metas(metas: &[CapabilityMeta]) -> Self {
        let mut decls = Self::default();
        for meta in metas {
            decls.declared.insert(meta.name.clone(), meta.behaviors);
        }
        decls
    }

    /// 本调用声明的**全部**行为名（原样切片，交声明闸与效果端口选实现用）。
    pub fn behaviors_of(&self, wire_name: &str) -> &'static [&'static str] {
        let wire = wire_name.trim();
        if let Some(declared) = self.declared.get(wire) {
            return declared;
        }
        let short = wire.rsplit('/').next().unwrap_or_default();
        self.declared.get(short).copied().unwrap_or(&[])
    }
}

// ── 结果守卫（体积上限与归档滚动）──────────────────────────────

/// 单条工具结果 token 预算（默认 ≈ 32KB 文本），可由守卫实现覆盖。
pub const DEFAULT_TOOL_RESULT_TOKEN_CAP: usize = 8192;

/// 每个会话的存档目录保留的最新文件数（防无限累积；超出按修改时间淘汰最旧）。
pub const TOOL_ARCHIVE_KEEP: usize = 20;

/// 守卫结果：进上下文的那段文本与它是否被裁剪。
pub struct GuardedResult {
    /// 裁剪后的文本（已可安全放进上下文）。
    pub text: String,
    /// 是否被截断 / 存档。
    pub truncated: bool,
    /// 原始 token 估算。
    pub original_tokens: usize,
    /// 存档路径（若写入成功）。
    pub archive_path: Option<String>,
}

/// 单条结果的体积守卫（环 → 批实现的**读数口**）。
///
/// 守卫自己持有预算与存档目录，批实现只交文本、取回可入上下文的文本；「按什么
/// 口径数 token、存到哪」不进入批合同——tokenizer 与压缩链归控制面（D-297）。
pub trait ToolResultGuard: Send + Sync {
    /// 裁剪一条工具结果。
    fn guard(&self, text: &str) -> GuardedResult;
}

// ── 批输入 / 出参 / 提供者合同 ────────────────────────────────

/// 一次工具批的**全部输入**（环 → 批提供者）。
///
/// 字段全部按值：执行可能跨多个 await 点、进并发档，借用会把批编排钉死在调用方
/// 的栈帧上。事件出口 [`ExecEventSink`] 就是「结果广播回调」——批实现只写出口，
/// 不直接认识会话存储。
pub struct ToolBatchInput {
    /// 模型交回的调用列表（本批）。
    pub calls: Vec<TurnToolCallInfo>,
    /// 批首读数（闸门 / 风险 / 声明位 / 台账）。
    pub readings: BatchGateReadings,
    /// 结果守卫（体积上限与归档滚动）。
    pub guard: Arc<dyn ToolResultGuard>,
    /// 父节点查询源：本批调用节点的权威副本。
    pub context_messages: Vec<ChatNode>,
    /// 环的宿主插件（钩子与回落路由的入口）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本轮的请求上下文。
    pub ctx: Arc<dyn PluginInvokeRequest>,
    /// 事件出口（结果广播回到会话的唯一回调）。
    pub sink: ExecEventSink,
    /// 本轮的中止信号。
    pub abort: ExecAbortSignal,
}

impl fmt::Debug for ToolBatchInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolBatchInput")
            .field("calls", &self.calls.len())
            .field("context_messages", &self.context_messages.len())
            .finish_non_exhaustive()
    }
}

/// 一次工具批交回的**全部出参**。
///
/// 节点与派生事实同时交回：前者供会话镜像落库，后者（效果 / 熔断理由）供轮末
/// 写口落格。只交节点会让效果静默全丢——出参不交回，写方就无从落格。
#[derive(Default)]
pub struct ToolBatchOutcome {
    /// 工具结果子节点（role=Tool）。
    pub tool_messages: Vec<ChatNode>,
    /// ToolCall 父节点的完整终态快照。
    pub parent_updates: Vec<ChatNode>,
    /// 效果出参（每次终态调用一条）。
    pub effects: Vec<EffectInput>,
    /// 熔断出参（`Break` 的理由；`Refuse` 不入此列）。
    pub breaks: Vec<&'static str>,
}

/// 工具批形状的执行合同（[`crate::port::PortProvider::tool_batch`] 的返回面）。
///
/// 与 [`crate::port::ToolCallExecutor`] 同形：跨 `dyn` 边界走 `Arc<dyn …>`——trait
/// 对象在 Rust 里转不出去，执行面必须以方法形式落在合同对象上（D-298）。
#[async_trait]
pub trait ToolBatchExecutor: Send + Sync + 'static {
    /// 执行一批工具调用：按读数跑闸门 / 钩子 / 执行 / 收口，把节点与派生事实交回。
    async fn execute_batch(&self, input: ToolBatchInput) -> ToolBatchOutcome;
}

#[cfg(test)]
#[path = "tool_batch.test.rs"]
mod tests;
