//! 插件 API —— 插件**可以**依赖的宿主面。
//!
//! 只有四类东西住在这里，四者都不是机制实现、而是插件与宿主共用的词汇：
//!
//! - [`routes`]：跨插件调用路径（地址）常量
//! - [`events`]：跨插件事件名常量与共享事件行（写闸的共享臂）
//! - [`behaviors`]：能力自声明的行为名（内核只装开放集合，不认识名字）
//! - [`effects`]：能力效果的实现端口（`ToolBehavior`）与草稿（`FactDraft`）
//! - [`context`]：能力贡献每轮上下文的端口（`ContextProvider`）与段身份
//! - [`hooks`]：`fire_hook` 助手与 hook 事件枚举
//! - [`declarations`]：声明表的链接期提交口（事件 / 事实 / 视图 / 端口 / 执行体 / 行）
//! - [`views`]：声明视图的跨插件读口（按 owner 公开的名字读纯折叠）
//! - [`view_names`]：`shared_views` 清单生成物（哪些视图公开可读）
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
pub mod view_names;
pub mod views;

pub use context::{
    providers as context_providers, section_meta, sections as context_sections, Cadence,
    ContextProvider, ContextRequest, ContextSection, DelegateVerdict, LiveInputs, RecallFacts,
    RecallLine,
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
pub use view_names::SharedViewDecl;
pub use views::{
    read_all_from as read_shared_view_all, read_from as read_shared_view, shared as shared_views,
    ViewRead, ViewUnavailable,
};
