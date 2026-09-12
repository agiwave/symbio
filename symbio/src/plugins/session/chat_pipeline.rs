//! 会话能力收集管线（session 插件内部设施）
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
//! ## 为什么放在 session 插件
//!
//! 本管线的唯一驱动方是 session 编排器：子智能体（agent 插件的 `agent_run`）需要
//! 嵌套会话时，经统一路由 `session/chat/send` 复用本管线，而非由 agent 插件直接
//! 调用——管线不再是跨插件共享设施，故从 `symbio_core` 下沉到 session 插件内部。
//!
//! 仍留在 `symbio_core` 的是**收集期错误通道**（`capability_error.rs`：写侧为任意
//! 参与 traverse 的插件、读侧为 session 编排方，属跨插件契约）；本文件仅消费。

use crate::symbio_core::{
    init_error_bucket, CapabilityVisitor, DefaultToolVisitor, InvokeRequest, InvokeRequestExt,
    Plugin, CAPABILITY_ERRORS, PATH, TRAVERSE_AVAILABLE_TOOLS,
};
use std::sync::Arc;

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
/// - 每次调用都返回**全新**的 `DefaultToolVisitor`——能力实例可能持有本次请求
///   相关的状态（人格摘要、智能体列表等），不能跨请求复用。
/// - `traverse` 失败只记录日志，不中断会话：单个插件故障不应让整个会话不可用。
/// - 父插件缺失（单插件内嵌场景）时返回空管理器，而非报错。
pub async fn collect_capabilities(
    parent: Option<&Arc<dyn Plugin>>,
    ctx: &Arc<dyn InvokeRequest>,
) -> Arc<dyn CapabilityVisitor> {
    let manager: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());

    let Some(parent) = parent else {
        return manager;
    };

    let traverse_ctx = ctx.fork();
    traverse_ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    traverse_ctx.set(crate::symbio_core::CAPABILITY_VISITOR, manager.clone());
    init_error_bucket(&traverse_ctx);

    if let Err(e) = parent
        .clone()
        .traverse(String::new(), traverse_ctx.clone())
        .await
    {
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

// 错误通道（CapabilityError / CAPABILITY_ERRORS / report_error / take_errors）
// 见 `crate::symbio_core::capability_error`：写侧是任意参与 traverse 的插件，
// 读侧是 session 编排方（orchestrator 在收集结束后 `take_errors` 统一裁决）。

/// 把能力管理器挂到请求上下文，供 model 插件的 chat_loop / tool_executor 取用。
///
/// 单独抽出的意义：让"收集"与"挂载"两件事在调用点显式成对出现，
/// 避免漏挂导致 `Tool not found` 这类只在运行期才暴露的问题。
pub fn attach_capabilities(ctx: &Arc<dyn InvokeRequest>, manager: Arc<dyn CapabilityVisitor>) {
    ctx.set(crate::symbio_core::CAPABILITY_VISITOR, manager);
}
