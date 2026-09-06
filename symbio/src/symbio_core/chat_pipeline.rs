//! 会话能力收集管线（跨插件共享设施）
//!
//! ## 背景
//!
//! 重构前，会话的工具集由 **agent 插件**在 `agent/chat` 路由里独家装配：
//! session 把 chat 请求整个转交给 agent，agent 再 `parent.traverse` 收工具、
//! 装 system prompt、最后转交 model。这让 agent 插件成为会话链路上的**特权节点**——
//! 其它插件（local / web / mcp / skill）都只是 `traverse` 的参与者，唯独 agent 既
//! 参与 `traverse` 又独占 chat 路由，且 session 必须选出一个 agent 才能对话。
//!
//! ## 重构后
//!
//! **会话编排权归 session 插件**。`traverse(TRAVERSE_AVAILABLE_TOOLS)` 是唯一且
//! 统一的工具贡献机制，所有插件（含 agent）在同一机制下按相同契约向会话附加能力：
//!
//! ```text
//! session ──collect_capabilities(parent, ctx)──▶ parent.traverse(available_tools)
//!                                                  ├─ local   : 文件/搜索/shell
//!                                                  ├─ web     : 搜索/抓取
//!                                                  ├─ mcp     : 外部 MCP 工具
//!                                                  ├─ skill   : 技能
//!                                                  └─ agent   : 仅当 ctx[AGENT_ID] 存在时贡献
//!                                                               （身份/认知/子智能体）
//! ```
//!
//! agent 是否贡献工具完全由 `ctx[AGENT_ID]` 决定——**不选择 agent 的会话照常运行**，
//! 只是没有智能体相关的工具与人格。
//!
//! ## 为什么放在 `symbio_core`
//!
//! session 插件与 agent 插件（子智能体 `agent_run` 需要起嵌套会话）都要走同一条
//! 管线，而插件之间不可见，故上浮为共享设施。

use crate::symbio_core::{
    CapabilityManager, DefaultToolManager, InvokeRequest, InvokeRequestExt, Plugin, SymbioKey,
    PATH, TRAVERSE_AVAILABLE_TOOLS,
};
use std::sync::Arc;
use tokio::sync::Mutex;

/// 向所有插件广播"贡献工具"，返回装配好的能力管理器。
///
/// ## 契约
///
/// 调用方需在 `ctx` 中预先设置好各插件判定所需的上下文键：
/// - `WORKDIR`（可选）：工作目录，决定 local / skill / agent 的作用域
/// - `SESSION_ID`：会话标识
/// - `AGENT_ID`（**可选**）：选定智能体。缺省时 agent 插件不贡献任何工具
///
/// ## 语义
///
/// - 每次调用都返回**全新**的 `DefaultToolManager`——能力实例可能持有本次请求
///   相关的状态（人格摘要、智能体列表等），不能跨请求复用。
/// - `traverse` 失败只记录日志，不中断会话：单个插件故障不应让整个会话不可用。
/// - 父插件缺失（单插件内嵌场景）时返回空管理器，而非报错。
pub async fn collect_capabilities(
    parent: Option<&Arc<dyn Plugin>>,
    ctx: &Arc<dyn InvokeRequest>,
) -> Arc<dyn CapabilityManager> {
    let manager: Arc<dyn CapabilityManager> = Arc::new(DefaultToolManager::new());

    let Some(parent) = parent else {
        return manager;
    };

    let traverse_ctx = ctx.fork();
    traverse_ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    traverse_ctx.set(crate::symbio_core::CAPABILITY_MANAGER, manager.clone());
    init_error_bucket(&traverse_ctx);

    if let Err(e) = parent.clone().traverse(String::new(), traverse_ctx.clone()).await {
        crate::plugin_warn!(
            "core",
            "collect_capabilities: traverse 失败（工具集可能不完整）: {:?}",
            e
        );
    }

    // 把子 ctx 上收集到的错误回写到调用方的 ctx，供编排方判定。
    // （`fork()` 会拷贝 extensions 快照，子 ctx 后续的写入不会自动回流）
    if let Some(bucket) = traverse_ctx.get(CAPABILITY_ERRORS) {
        ctx.set(CAPABILITY_ERRORS, bucket);
    }

    manager
}

// ═══════════════════════════════════════════════════════════════════════════
// 收集期错误通道
// ---------------------------------------------------------------------
// `Composite::traverse` 会吞掉子插件返回的 Err（`let _ = plugin.traverse(...)`），
// 单插件因此**无法**靠返回值让整次能力收集失败。但有些失败是致命的、必须让会话
// 立刻中止并明确报错——最典型的就是"会话绑定了一个不存在的智能体"。
//
// 解决：收集期错误写入 ctx 上的一个共享桶，由编排方（session）在收集结束后统一
// 取用判定。这是零耦合的：桶由 symbio_core 提供，任何插件都能报，编排方统一裁决。
// ═══════════════════════════════════════════════════════════════════════════

/// 能力收集期错误
#[derive(Debug, Clone)]
pub struct CapabilityError {
    /// 报错插件名（用于日志与错误信息定位）
    pub plugin: String,
    /// 人类可读的错误描述（会直接展示给用户）
    pub message: String,
}

/// 收集期错误桶在 `InvokeRequest` 中的类型安全键
pub struct CapabilityErrorsKey;

impl SymbioKey for CapabilityErrorsKey {
    type Value = Arc<Mutex<Vec<CapabilityError>>>;
    fn name(&self) -> &'static str {
        "capability_errors"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "capability_errors".to_string()
    }
}

/// 收集期错误桶键常量
pub const CAPABILITY_ERRORS: CapabilityErrorsKey = CapabilityErrorsKey;

/// 初始化错误桶（由 `collect_capabilities` 调用；重复调用无副作用）
pub fn init_error_bucket(ctx: &Arc<dyn InvokeRequest>) {
    if ctx.get(CAPABILITY_ERRORS).is_none() {
        ctx.set(CAPABILITY_ERRORS, Arc::new(Mutex::new(Vec::new())));
    }
}

/// 插件在 `traverse` 中报告一个**致命**收集错误。
///
/// 只用于"会话无法继续"的硬错误（如选定的智能体不存在）。
/// 可降级的软故障（某个 MCP server 连不上）应当只记日志，不调用本函数。
pub async fn report_error(ctx: &Arc<dyn InvokeRequest>, plugin: &str, message: impl Into<String>) {
    init_error_bucket(ctx);
    if let Some(bucket) = ctx.get(CAPABILITY_ERRORS) {
        bucket.lock().await.push(CapabilityError {
            plugin: plugin.to_string(),
            message: message.into(),
        });
    }
}

/// 取出并清空所有收集期错误
pub async fn take_errors(ctx: &Arc<dyn InvokeRequest>) -> Vec<CapabilityError> {
    match ctx.get(CAPABILITY_ERRORS) {
        Some(bucket) => std::mem::take(&mut *bucket.lock().await),
        None => Vec::new(),
    }
}

/// 把能力管理器挂到请求上下文，供 model 插件的 chat_loop / tool_executor 取用。
///
/// 单独抽出的意义：让"收集"与"挂载"两件事在调用点显式成对出现，
/// 避免漏挂导致 `Tool not found` 这类只在运行期才暴露的问题。
pub fn attach_capabilities(ctx: &Arc<dyn InvokeRequest>, manager: Arc<dyn CapabilityManager>) {
    ctx.set(crate::symbio_core::CAPABILITY_MANAGER, manager);
}
