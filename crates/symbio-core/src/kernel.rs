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
//! C2b 之后**全部封闭语法**都已住 `symbio-kernel` crate（D-188），本文件只剩转出：
//! CS-001（形状）与 CS-004（插件 id 字面量）的文件集标记因此整份迁到
//! `crates/symbio-kernel/src/lib.rs`，这里不再有判定范围。

// ── Event 信封与 Entity×Verb（DE-023 §5） ──
pub use super::event::{Entity, Event, EventEnvelope, Seq, Verb};

// ── 主体规格五字段与模式闭集 ──
// 数据面住内核 crate；本机的授权读法（平凡值 / 模式派生 / 能力位）是 core 的
// 自由函数（`trivial_actor_spec` / `pattern_for_principal` / `actor_can_*`），见
// `actors::spec` —— 部署表不是语法（D-099）。
pub use symbio_kernel::actors::Scope;
pub use symbio_kernel::actors::{ActorSpec, Pattern};

// ── Store / Projection / Budget ──
pub use super::projection::Projection;
pub use super::store::Store;
pub use super::view::Budget;

// ── 声明容器（Builder / PluginDeclarations）与 seal（Frozen） ──
pub use super::decl::ring;
pub use super::decl::{ActorDecl, Builder, FactDecl, Frozen, PluginDeclarations, ViewDecl};

// ── 五个封闭语法 trait：Store 在上，其余四个 ──
pub use super::capability::{Capability, Registry};
pub use super::llm::model_provider::ModelProvider;
pub use super::vdfs::VdfsProvider;

// ── 协议五格的事件名（环七步的输入面） ──
pub use super::event::{
    EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};
