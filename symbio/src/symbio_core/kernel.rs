//! 内核门面：**封闭语法**的唯一汇合口（DE-023 A2 / §5）
//!
//! 这里只重导出、不拥有声明：Event 信封、Entity×Verb、`ActorSpec` 五字段、
//! `Pattern` / `Scope`、声明容器与 `seal`、环七步、5 个封闭语法 trait。**词汇与行为
//! 仍住各自的域**——加事实 / 视图 / 策略 / 插件的改动不应落到本文件（DE-023 D-177）。
//!
//! 根 [`mod.rs`](mod.rs) 用 `pub use kernel::*` 把它平铺出去，所以对外路径不变
//! （仍是 `symbio_core::<符号>`）；本模块私有，外部写不出 `symbio_core::kernel::…`。
//!
//! ## 内核文件集
//!
//! 下面的 `core-kernel-files:` 标记是 CS-001（形状）与 CS-004（插件 id 字面量）
//! 的**判定范围**，由 `scripts/core-shape-audit.mjs` 读取。文件集是「声明落在哪个
//! 文件」而不是「域」：`capability` / `vdfs` 等域仍有大量词汇与行为，只有 trait
//! 声明那一处算语法。C2a 已把 `event` / `view` / `store` / `projection::Projection` /
//! `decl` / `LatencyTier` 搬进 `symbio-kernel` crate（D-188），那部分文件集标记住
//! `symbio_kernel/src/lib.rs`；本文件只剩仍住 core 的语法声明。
// core-kernel-files: actors/spec.rs capability/mod.rs llm/model_provider.rs vdfs/provider.rs

// ── Event 信封与 Entity×Verb（DE-023 §5） ──
pub use super::event::{Entity, Event, EventEnvelope, Seq, Verb};

// ── 主体规格五字段与模式闭集 ──
pub(crate) use super::actors::Scope;
pub use super::actors::{ActorSpec, Pattern};

// ── Store / Projection / Budget ──
pub use super::projection::Projection;
pub use super::store::Store;
pub use super::view::Budget;

// ── 声明容器（Builder / PluginDeclarations）与 seal（Frozen） ──
pub(crate) use super::decl::ring;
pub(crate) use super::decl::{ActorDecl, Builder, FactDecl, Frozen, PluginDeclarations, ViewDecl};

// ── 五个封闭语法 trait：Store 在上，其余四个 ──
pub use super::capability::{Capability, Registry};
pub use super::llm::model_provider::ModelProvider;
pub use super::vdfs::VdfsProvider;

// ── 协议五格的事件名（环七步的输入面） ──
pub(crate) use super::event::{
    EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};
