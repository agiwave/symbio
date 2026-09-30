//! `actors` —— 主体（v2 ②，[plan/01 §4](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 形状（01 §4 的五字段，一字未增）
//!
//! `model` 与 `context` **不是字段**：`model` 是 `pattern` 的函数（Decider→无、
//! Reasoner→LLM、Translator→小模型），`context` 是 `projection` 的参数——
//! 能由别的东西算出来的就不占字段（J1）。
//!
//! ## 依赖纪律（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md) ② 行）
//!
//! 只接收**类型化输入**（事件切片 / `View`），**产出事件**——不持有任何其他模块的
//! 句柄。「想调用也拿不到对方的句柄」是构造保证，不是约定。
//!
//! ## S1 范围（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 3 步：零 LLM 彩排）
//!
//! 只落 `Decider`（规则应答，`pattern` 的**平凡值**）——零 LLM 也能跑通完整闭环
//! （[roadmap/S01 §4](../../../../docs/plan/roadmap/S01-最小闭环.md)：平凡值下系统
//! 必须完整运行）。`Reasoner` / `Translator` 的**产出路径**在 S2 / S5 接入；
//! `capabilities` 暂为字符串数据（能力名），枚举闭集与 grants 校验归 S3 ⑥
//! `governance`——本域不抢。
//!
//! ## 名字的两处出处
//!
//! - `ActorSpec` / `Pattern` / `Scope`：[plan/01 §4](../../../../docs/plan/01-核心架构.md)
//!   的冻结契约名（名字先于模块存在，判据同 `schemas`）；
//! - `Decider`：[plan/05 §4](../../../../docs/plan/05-模块架构.md) S8 的反射档判定者，
//!   S1 先以规则应答形态落地。

use crate::symbio_core::adapters::{AdapterError, FullModel, LlmAdapter};
use crate::symbio_core::event::Event;

/// 主体模式（[plan/01 §4](../../../../docs/plan/01-核心架构.md)：机制，**3 个封顶**）。
///
/// 模式选择的判据是函数不是清单：可写成确定性规则 → [`Pattern::Decider`]；
/// 只需重述已有事实 → [`Pattern::Translator`]；否则 → [`Pattern::Reasoner`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// 规则 / 代码：确定，毫秒级（意图判定、就绪判定、权限、熔断、仲裁）。
    Decider,
    /// LLM：非确定，秒–分钟（规划、生成、语义判定、工具调用）。
    Reasoner,
    /// 事件 → 另一种表示：确定，百毫秒。**不创造信息**（输出可追溯到输入事件）。
    Translator,
}

/// 作用域（[plan/01 §8](../../../../docs/plan/01-核心架构.md)）。
///
/// `root` 是**一等输入**，不是"没有子智能体"的特例（03 §2 推论：
/// 不出现 `if (children.is_empty())` 分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Root,
    /// 子智能体：挂在其父主体的作用域下。
    Child(String),
}

/// 主体规格（[plan/01 §4](../../../../docs/plan/01-核心架构.md) 的 `ActorSpec`，五字段）。
#[derive(Debug, Clone)]
pub struct ActorSpec {
    /// 身份（数据，无限增长——如 `"agent:main"`）。
    pub principal: String,
    /// 三种模式之一（机制，封顶）。
    pub pattern: Pattern,
    /// 能力名（数据）。S3 之前不做枚举闭集与校验（归 ⑥ `governance`）。
    pub capabilities: Vec<String>,
    /// I3 时延预算（毫秒）。
    pub budget_ms: u64,
    /// 作用域（递归）。
    pub scope: Scope,
}

impl ActorSpec {
    /// S01 的平凡值主体（[roadmap/S01 §4](../../../../docs/plan/roadmap/S01-最小闭环.md)）：
    /// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
    pub fn trivial(principal: impl Into<String>) -> Self {
        ActorSpec {
            principal: principal.into(),
            pattern: Pattern::Decider,
            capabilities: Vec::new(),
            budget_ms: 60_000,
            scope: Scope::Root,
        }
    }
}

/// 规则未命中——**不是错误，是兜底的触发条件**。
///
/// [roadmap/S01 §2](../../../../docs/plan/roadmap/S01-最小闭环.md)：兜底话术也是一条
/// 普通事件（`chat.assistant.fallback`，网格里已有的一格，**不是特殊通道**）。
/// 因此「Decider 答不出」在类型上就是 `Err(DeciderMiss)`，调用方据此产兜底事件——
/// 它永远逃不出审计（I2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeciderMiss {
    /// 未命中的输入摘要（入兜底事件的载荷，可观测）。
    pub utterance: String,
}

/// 反射档规则应答器（`pattern = Decider` 的 S1 形态）。
///
/// **只接收事件切片**（View 的原始形态），产出应答文本——无任何模块句柄。
/// 确定性：同一事件序列 ⇒ 同一应答（N1 在整条链路上成立的前提）。
///
/// 规则表是**数据**：加规则不加分支（同事件网格的"加名字不加枚举"）。
pub struct Decider {
    rules: Vec<(&'static str, &'static str)>,
}

impl Decider {
    /// 规则表驱动构造：`(子串匹配, 应答)` 逐条尝试，**首条命中即返回**。
    pub fn new(rules: Vec<(&'static str, &'static str)>) -> Self {
        Decider { rules }
    }

    /// S01 彩排用的最小规则表（内容来自
    /// [`docs/plan/verify/latency_gate.rs`](../../../../docs/plan/verify/latency_gate.rs) 的 `RuleEngine`）。
    pub fn rehearsal() -> Self {
        Decider::new(vec![
            ("你好", "你好，我能做什么？"),
            ("帮我", "我可以帮你查资料、跑任务、写东西。"),
        ])
    }

    /// 对**最后一条用户消息**应答。规则未命中 ⇒ [`DeciderMiss`]（调用方产兜底事件）。
    ///
    /// 输入是事件切片而非裸文本：Decider 自己从事件里找 `user.message`——
    /// 「收到」这个端点（[roadmap/S01 §1](../../../../docs/plan/roadmap/S01-最小闭环.md)）
    /// 由此成为它的输入契约，而不是调用方的口头约定。
    pub fn respond(
        &self,
        events: &[crate::symbio_core::event::Event],
    ) -> Result<String, DeciderMiss> {
        let utterance = events
            .iter()
            .rev()
            .find(|e| e.kind == crate::symbio_core::event::EVENT_USER_MESSAGE)
            .map(|e| {
                e.payload
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default();
        for (needle, reply) in &self.rules {
            if utterance.contains(needle) {
                return Ok((*reply).to_string());
            }
        }
        Err(DeciderMiss { utterance })
    }
}

/// 生成档主体（`pattern = Reasoner` 的 S2 形态，[plan/01 §4](../../../../docs/plan/01-核心架构.md)）。
///
/// ## 闸门在签名上
///
/// [`Reasoner::reply`] 要求 `&FullModel` 令牌——**反射 / 快速档拿不到这个令牌**，
/// 所以「反射层调模型」不是被检测到，而是**编译不过**（J3，[plan/01 §10](../../../../docs/plan/01-核心架构.md)
/// 第 4 条语义）。令牌由 ⑤ `adapters` 的 [`crate::symbio_core::adapters::TokenIssuer`]
/// 按档位签发，装配时注入；`Reasoner` 自身不持有任何适配器句柄。
pub struct Reasoner;

impl Reasoner {
    /// 生成答复：从事件切片取最后一条用户消息作输入，经端口生成。
    ///
    /// 失败形态是 [`AdapterError`](crate::symbio_core::adapters::AdapterError)
    /// （不是 [`DeciderMiss`]）——**调用方必须产出 `chat.assistant.fallback` 事件**
    /// （I3 到点必答：禁止静默超时，[plan/01 §10](../../../../docs/plan/01-核心架构.md) 第 2 条）。
    pub async fn reply(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
    ) -> Result<String, AdapterError> {
        let prompt = events
            .iter()
            .rev()
            .find(|e| e.kind == crate::symbio_core::event::EVENT_USER_MESSAGE)
            .map(|e| {
                e.payload
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default();
        llm.generate(tok, &prompt).await
    }
}

/// 检索 Translator（`pattern = Translator` 的 S5 形态，[plan/01 §9.2](../../../../docs/plan/01-核心架构.md)）。
///
/// **Translator 不创造信息**：召回内容原样来自 `RecallView`（③ 的投影产出），
/// 本体的职责只是把「读视图」落成「一条事实」——`memory.recalled` 事件
/// （`memory × asserted` 格子，带溯源指向触发它的事件）。检索是
/// 「读视图 → 产出事实」，这正是 Actor 定义对 Translator 的要求。
pub struct RecallTranslator;

impl RecallTranslator {
    /// 把召回结果固化为一条 `memory.recalled` 事件。
    ///
    /// `trigger_seq`：触发本次检索的事件 seq（溯源锚——I2：断言类必带溯源）。
    pub fn recalled_event(
        &self,
        view: &crate::symbio_core::view::RecallView,
        trigger_seq: u64,
    ) -> Event {
        let top = view
            .entries
            .first()
            .map(|e| e.content.as_str())
            .unwrap_or("");
        Event::pending(
            format!("recalled-{trigger_seq}"),
            crate::symbio_core::event::EVENT_MEMORY_RECALLED,
            crate::symbio_core::event::Entity::Memory,
            crate::symbio_core::event::Verb::Asserted,
            0,
            "agent:main",
        )
        .with_produced_by(trigger_seq)
        .with_payload(serde_json::json!({
            "found": view.entries.len(),
            "top": top,
        }))
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
