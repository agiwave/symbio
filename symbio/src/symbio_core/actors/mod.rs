//! `actors` —— 主体（v2 ②，[plan/01 §4](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构")）。
//!
//! ## 形状（01 §4 的五字段，一字未增）
//!
//! `model` 与 `context` **不是字段**：`model` 是 `pattern` 的函数（Decider→无、
//! Reasoner→LLM、Translator→小模型），`context` 是 `projection` 的参数——
//! 能由别的东西算出来的就不占字段（J1）。
//!
//! ## 依赖纪律（[plan/05 §3.1](../../../../docs/plan/PL-005-模块架构.md "PL-005-模块架构") ② 行）
//!
//! 只接收**类型化输入**（事件切片 / `View`），**产出事件**——不持有任何其他模块的
//! 句柄。「想调用也拿不到对方的句柄」是构造保证，不是约定。
//!
//! ## S1 彩排（[plan/04 §3](../../../../docs/plan/PL-004-工程落地.md "PL-004-工程落地") 第 3 步：零 LLM 彩排）
//!
//! 闭环彩排跑在 ⑤ `adapters` 的确定性桩上（零 LLM、毫秒级），链路一条不少：
//! 用户消息入库 → 应答 → final/fallback 入库 → 三查。规则应答的语义由
//! [`classify`](../../../../docs/archive/06-会话响应性落地.md) 的规则表以更严纪律承接，
//! 本域只留**模式名**；`Reasoner` / `Translator` 的**产出路径**在 S2 / S5 接入；
//! `capabilities` 暂为字符串数据（能力名），枚举闭集与 grants 校验归 S3 ⑥
//! `governance`——本域不抢。
//!
//! ## 名字的两处出处
//!
//! - `ActorSpec` / `Pattern` / `Scope`：[plan/01 §4](../../../../docs/plan/PL-001-核心架构.md "PL-001-核心架构")
//!   的冻结契约名（名字先于模块存在，判据同 `schemas`）；
//! - [`Pattern::Decider`]：[plan/05 §4](../../../../docs/plan/PL-005-模块架构.md "PL-005-模块架构") S8 的
//!   反射档判定者——模式名在此，规则应答不在（见上）。

mod autonomic;
mod commitment;
mod conation;
mod control;
mod reasoner;
mod recall;
mod skill;
mod spec;

pub use self::autonomic::AutonomousInitiator;
pub use self::commitment::commitment_events;
pub use self::conation::{ConationCandidate, ConationPolicy, IntentGate};
pub use self::control::{CircuitBreaker, GateDecision, Preemption, PreemptionDecider};
pub use self::reasoner::render_messages;
pub use self::recall::recalled_event;
pub use self::skill::{SkillCompiler, SkillRoute, SkillRouter};
pub use self::spec::{ActorSpec, Pattern, Scope};

// 仅测试面：这些符号在非测试构建里没有跨域消费者（用点就在各自的子模块内），
// 但 `mod.test.rs` 经 `use super::*` 取用，故只在 `cfg(test)` 出根——与
// `symbio_core/mod.rs` 的 `#[cfg(test)] pub(crate) use event::{…}` 同一形态。
#[cfg(test)]
pub use self::commitment::CommitmentKeeper;
#[cfg(test)]
pub use self::conation::{GateWarrant, IntentDecision};
#[cfg(test)]
pub use self::recall::RecallTranslator;
// `Scope` 已随 `ActorSpec` / `Pattern` 常驻出根（上文）：`ActorSpec` 的生产构造点
// （会话插件的 `request_actor_spec`）要以 `Scope::Root` 立规格，不再只是测试面。

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
