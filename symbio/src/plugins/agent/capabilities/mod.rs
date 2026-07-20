//! Agent 能力模块
//!
//! 提供 agent 插件的核心能力实现。
//!
//! ## 核心能力体系
//!
//! 采用统一认知工具 + 操作级拆分架构：
//!
//! ### 对外能力（3个）
//! - **agent_chat**: 对话能力
//! - **agent_cognition**: 统一认知工具（26 个操作，覆盖 5 个认知域）
//!   - 每个操作是独立的 `.rs` 文件，实现 `CognitionOp` trait
//!   - 通过 `OpRegistry` 注册表统一分发
//! - **agent_create**: 创建智能体
//!
//! ### 设计原则
//! - **操作级拆分**：每个操作独立文件，便于扩展和测试
//! - **统一入口**：LLM 只需理解 1 个工具 + 操作列表
//! - **注册机制**：操作通过 `OpRegistry` 注册，无需硬编码 match
//!
//! ### 注册机制
//!
//! - **capability 注册**：每个 capability 通过 `submit_object_creator!` 宏自注册
//! - **操作注册**：每个操作通过 `submit_cognition_op!` 宏自注册到全局 `OpRegistry`（基于 inventory）

// 核心能力
mod chat; // 对话能力

// 统一认知工具（合并 memory/reason/learn/plan/metacognition）
mod cognition;

// 操作级拆分模块（每个操作独立实现，统一注册）
pub(crate) mod ops;

// 内部辅助模块
mod graph;

use crate::plugins::agent::manager::AgentProfile;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::InvokeRequest;
use crate::symbio_core::SymbioKey;
use std::sync::Arc;

// ─── 能力模块配置常量 ───

// 推理相关常量

/// 默认置信度阈值
pub(crate) const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.7;

// 记忆类型

/// 工作记忆类型标识
#[allow(dead_code)]
pub(crate) const MEMORY_TYPE_WORKING: &str = "working";

// 工作记忆 TTL（v10 新增）

/// 工作记忆默认 TTL（秒）
///
/// v10 设计：把 `save_working` 与 `save` 的语义统一通过 TTL 区分：
/// - `save`：永久记忆（无 expires_at）
/// - `save_working`：临时记忆（默认 24 小时，自动过期）
#[allow(dead_code)]
pub(crate) const WORKING_MEMORY_DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// 工作记忆最大 TTL（秒）：7 天
///
/// 防止 LLM 误传极大值导致"永久工作记忆"语义不清。
#[allow(dead_code)]
pub(crate) const WORKING_MEMORY_MAX_TTL_SECS: u64 = 7 * 24 * 60 * 60;

// ═══════════════════════════════════════════════════════════════════════════
// Agent 能力工厂上下文——承载工厂构造所需的所有运行时数据
// ----------------------------------------------------------------------
// 设计要点：
// - 各 capability 模块的工厂函数统一通过 `InvokeRequest` 取出此上下文
// - 工厂本身保持 `fn(Arc<dyn InvokeRequest>) -> Arc<dyn Capability>` 签名，
//   与 `submit_object_creator!` 现有宏协议**完全一致**
// - `chat` 工厂需要额外的 `agents` 列表，由本结构一并承载
// - `parse` / `format` 不适用（结构体无法从字符串恢复），按 `ParentKey` 的
//   模式返回 `None` / 占位即可——本键仅用于进程内类型安全的存取
// ═══════════════════════════════════════════════════════════════════════════

/// 能力工厂构造上下文：当前 `AgentPlugin` 实例 + 当前 workdir 下的智能体列表
#[derive(Clone)]
pub(crate) struct AgentCapabilityContext {
    pub(crate) plugin: Arc<AgentPlugin>,
    pub(crate) agents: Vec<AgentProfile>,
}

/// 能力工厂上下文在 `InvokeRequest` 中的类型安全键
pub(crate) struct AgentCapabilityContextKey;

impl SymbioKey for AgentCapabilityContextKey {
    type Value = Arc<AgentCapabilityContext>;
    fn name(&self) -> &'static str {
        "agent_capability_context"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        String::new()
    }
}

/// 上下文键常量——供 `plugin.rs` 与各 capability 工厂共用
pub(crate) const AGENT_CAPABILITY_CONTEXT: AgentCapabilityContextKey = AgentCapabilityContextKey;

/// 从 InvokeRequest 中提取 AgentCapabilityContext
///
/// 统一处理 7 处相同的 expect 模式，减少重复代码。
/// Panics: 如果 plugin.rs 未注入 AgentCapabilityContext（编程错误，应尽早暴露）。
pub(crate) fn get_capability_context(ctx: &dyn InvokeRequest) -> Arc<AgentCapabilityContext> {
    ctx.get_raw(AGENT_CAPABILITY_CONTEXT.name())
        .and_then(|any| any.downcast::<AgentCapabilityContext>().ok())
        .expect("AgentCapabilityContext missing in InvokeRequest — plugin.rs must inject it before traverse")
}
