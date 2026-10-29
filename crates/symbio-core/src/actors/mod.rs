//! `actors` —— 主体（②，[DE-001 §4](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 形状（01 §4 的五字段，一字未增）
//!
//! `model` 与 `context` **不是字段**：`model` 是 `pattern` 的函数（Decider→无、
//! Reasoner→LLM、Translator→小模型），`context` 是 `projection` 的参数——
//! 能由别的东西算出来的就不占字段（J1）。
//!
//! ## 依赖纪律（[DE-018 §3.1](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构") ② 行）
//!
//! 只接收**类型化输入**（事件切片 / `View`），**产出事件**——不持有任何其他模块的
//! 句柄。「想调用也拿不到对方的句柄」是构造保证，不是约定。
//!
//! ## 本域只装「必须留在核内」的策略
//!
//! 单消费方的策略实现归 owner 插件（D-189 / D-190）：调度在 `crates/symbio-plugin-loop/src/task_events`、
//! 技能编译与路由在 `crates/symbio-plugin-loop/src/skill_events`、检索 Translator 在
//! `crates/symbio-plugin-loop/src/memory_events`、自主层与意欲闸门在 `crates/symbio-plugin-session/src/transcript/autonomous`；
//! 抢占判定在 `crates/symbio-plugin-session/src/transcript/inbox`、外部执行闸门在
//! `crates/symbio-plugin-loop/src/tools/dispatch/gates`。
//!
//! 留在本域的三类各有判据：
//!
//! - `commitment`：写侧成格 / 读侧翻译的**配对契约**（写方在 loop、读方在 session）——
//!   搬任一侧都要把另一侧切成第二套规则（D-187 第 1 条）；
//! - `control`：只剩淘汰裁决——写侧在 loop 轮末、读侧翻译（`verdict_grants_changes`）
//!   在 session 工具层，**真的跨两个 owner**（D-190）；
//! - `spec`：`ActorSpec` 的数据面在 `symbio_kernel`，读本机部署表的动作（模式派生 /
//!   能力位判定）留宿主（D-188）。
//!
//! `SKILL_TAG` 是 loop 写、session 读的共享词汇，按 D-177 / D-182 留在本域。

mod commitment;
mod control;
mod skill;
mod spec;

pub use self::commitment::{commitment_events, commitment_records};
pub use self::control::EliminationDecider;
pub use self::skill::SKILL_TAG;
// 数据面随 D-188 C2b 住内核 crate；本域只剩读部署表的自由函数（见 `spec` 模块文档）。
pub use self::spec::{
    actor_can_assert_verification, actor_can_assign_work, actor_can_judge_intent,
    actor_can_land_artifact, actor_can_land_speech, pattern_for_principal,
};
// `trivial_actor_spec` 是**测试面**的平凡值构造（ADR-067：生产构造点按真实授权行立规格）。
#[cfg(test)]
pub(crate) use self::spec::trivial_actor_spec;

// 仅测试面：这些符号在非测试构建里没有跨域消费者（用点就在各自的子模块内），
// 但 `mod.test.rs` 经 `use super::*` 取用，故只在 `cfg(test)` 出根——与
// `symbio_core/mod.rs` 的 `#[cfg(test)] pub use event::{…}` 同一形态。
#[cfg(test)]
pub use self::commitment::CommitmentKeeper;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
