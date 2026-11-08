//! `actors` —— **主体面**：规格读法 · 承诺登记 · 淘汰裁决 · 技能词汇
//!
//! ## 这一域治什么
//!
//! `ActorSpec` 的**数据面**在 `symbio_kernel`（五字段，F3 冻结锚点）；本域装四件读它、
//! 写它、按它判的东西：
//!
//! | 模块 | 内容 | 谁消费 |
//! |---|---|---|
//! | `spec` | 本机部署表的读法——模式派生（`pattern_for_principal`）与能力位判定（`actor_can_*`） | 全部能力包 |
//! | `commitment` | 「谁向谁承诺了什么 / 是否履行」落成普通事件（`CommitmentKeeper`） | 写侧 `agent` · 读侧 `session` |
//! | `control` | 淘汰裁决（`EliminationDecider`）：声誉投影低于阈值 ⇒ 收回 `assign.work` | 写侧 `loop` 轮末 · 读侧 `session` 工具层 |
//! | `skill` | `SKILL_TAG`：`memory.encoded` 载荷里的技能标签（DE-041） | 写侧技能能力包 · 读侧 `session` 路由 |
//!
//! ## 为什么四块住在一起（D-279）
//!
//! 四块住在一个域里，判据只有一条（D-279）：**认不认识容器概念**。整个 `actors` 域一条
//! 都不认——`control` 读的是 `verdict × asserted` 这个**事实格**（内核语法），不是轮次、
//! 幕或容器；它和 `commitment` 是同一形状：一侧写格、一侧翻译格。
//!
//! 分居的代价是具体的：`control` / `skill` 在 `symbio-core` 时，那个 crate 只剩 322 行
//! 生产代码、27 个文件里 21 个是转发桩，却仍要向每个新读者解释一遍「这块为什么不和
//! `spec` / `commitment` 住一起」。答案是「因为它跨 owner」（D-187 / D-189 / D-190）——
//! 而 D-276 §2 已经判过：**跨 owner 是归属，不是住址**。归属由本文件与决策记录表达，
//! 拿 crate 表达归属，代价是插件为一个它根本不拥有的模块付一整层的编译账。

mod commitment;
mod control;
mod skill;
mod spec;

pub use self::commitment::{commitment_events, commitment_records};
pub use self::control::EliminationDecider;
pub use self::skill::SKILL_TAG;
pub use self::spec::{
    actor_can_assert_verification, actor_can_assign_work, actor_can_judge_intent,
    actor_can_land_artifact, actor_can_land_speech, pattern_for_principal,
};
// `trivial_actor_spec` 是**测试面**的平凡值构造（ADR-067：生产构造点按真实授权行立规格）。
#[cfg(test)]
pub(crate) use self::spec::trivial_actor_spec;

// 仅测试面：`CommitmentKeeper` 在非测试构建里没有跨域消费者（生产经 `commitment_events`
// 的四个格名），但 `mod.test.rs` 经 `use crate::actors::*` 取用。
#[cfg(test)]
pub use self::commitment::CommitmentKeeper;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
