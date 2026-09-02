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
mod chat; // 对话能力（子智能体委托）
mod identity; // 身份能力：把人格随工具说明送达 LLM

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

/// 新写入 CU 的默认置信度（save op 未显式传 confidence 时使用）
pub(crate) const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.7;

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
///
/// 重构说明（agent 降级为普通插件）：
/// - `agent_id` / `persona` 为 `Option`——**会话未选择智能体时**，agent 插件在
///   `traverse` 阶段就整体早退，根本不会构造本上下文；这里保留 Option 是为了让
///   "是否挂载了智能体"在类型层面显式可判，而不是靠空字符串约定。
/// - `persona` 由 `traverse`（异步）预先渲染后注入：能力工厂签名是同步的
///   （`fn(Arc<dyn InvokeRequest>) -> Arc<dyn Capability>`），而人格渲染要访问
///   存储，只能在异步阶段算好再塞进上下文。
#[derive(Clone)]
pub(crate) struct AgentCapabilityContext {
    pub(crate) plugin: Arc<AgentPlugin>,
    pub(crate) agents: Vec<AgentProfile>,
    /// 本次会话选定的智能体 id（`None` = 未选择智能体）
    pub(crate) agent_id: Option<String>,
    /// 预渲染的人格文本（身份 / 规则 / 策略 / 预算），供 `agent_identity` 说明嵌入
    pub(crate) persona: Option<String>,
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
