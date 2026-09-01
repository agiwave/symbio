//! Agent 核心接口层
//!
//! - **接口**子模块（types/store/traits/error/config/typed_unit）→ `mod`（私有）
//! - **实现**子模块：仅 `metrics` 因跨模块调用方存在 → `pub(crate) mod`
//!   其余（embedding_quant）保持 `mod`（仅 core 内部使用）
//!
//! 关系判定由 `CognitiveUnit::is_relation_prop()` 数据驱动，
//! 从 prop CU 的 `is_a` 含 `relation` + `prop_value_is_a` ∈ {cu, cu[]} 派生，
//! 不需要独立的注册表模块。

// ─── 接口子模块（私有——仅本 core 模块可见）───
mod config;
mod error;
/// 系统提示词预算分配器（[三层目标] 第 1/2/3 层的共享基础设施）
///
/// - 公开 `estimate_tokens` 给 `system_prompt::build`（动态构建 + 预算分配）
/// - 公开 `PromptBudget` / `BudgetUsage` 给系统提示词的"预算告警"段
///   （不再通过 op 暴露给 LLM——由系统主动驱动）
mod prompt_budget;
mod store;
mod traits;
mod typed_unit;

// ─── 实现子模块：跨模块调用方存在 → pub(crate) ───
pub(crate) mod default_tool_manager;

// ─── 实现子模块：跨模块调用方存在（types 提供 cu_fields/CuRef/generate_short_id 给 store/typed_unit/scaffold）→ pub(crate) ───
pub(crate) mod types;

// ─── 实现子模块：仅 core 内部使用 → mod ───
mod embedding_quant;

// ─── 公共 API 聚合 reexport ───
//
// 关键：所有 `pub use` 项必须在 `core` 模块内被**真实使用**至少一次，
// 否则 rustc 会报 `unused_imports` 警告。本 mod.rs 顶部不直接使用这些类型，
// 故在文件底部用 `#[cfg(test)] mod api_surface` 块做 public API 烟测——
// 既消除警告，又能在编译期验证公开接口可用。
pub use crate::symbio_core::providers::EmbeddingService;
pub use config::{AgentConfig, CognitionThresholds, StorageBackendType, StorageFormat};
pub use error::{AgentError, AgentResult};
pub use prompt_budget::{compute_cu_score, estimate_tokens, BudgetUsage, CuScore, PromptBudget};
pub use store::{
    cosine_similarity, evaluate_filter, AgentStore, FilterExpr, PageRequest, PageResult, StoreError,
};
pub use traits::CognitionContext;
pub use typed_unit::CognitiveUnit;
#[cfg(test)]
pub use types::new_cognitive_unit;
pub use types::{cu_from_json, now_secs, truncate_chars, unit_with_id, OperationResult};

/// 系统核心关系名（COGNITION.md §2.4 核心关系列表）
///
/// 这些名字来自 seed_cus.jsonl 中的 prop CU 数据声明，而非硬编码业务规则。
/// 运行时可通过新增 prop CU 扩展更多关系。
/// 主要消费方：`store/mindscape/scaffold.rs` 启动时的 prop 完整性校验。
pub const CORE_RELATION_NAMES: &[&str] = &[
    "is_a", "has", "part_of", "causes", "depends", "similar", "opposite", "related",
];

// ─── 公开 API 烟测：保证顶层 reexport 全部可用 ───
//
// 不通过 `#[allow(unused_imports)]` 抑制警告，而是**真实引用**每个 reexport
// 类型一次——任何被误删/重命名的项都会让本块编译失败，从而暴露问题。
// 测试代码已抽离到 `core/tests.rs`（独立文件，保持主文件简洁）
#[cfg(test)]
mod tests;
