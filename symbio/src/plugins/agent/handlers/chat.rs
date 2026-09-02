//! 子智能体会话执行入口
//!
//! ## 重构后的职责边界（重要）
//!
//! **顶层会话不再经过本 handler。** 前端会话统一由 session 插件编排
//! （`plugins/session/orchestrator.rs`）：session 自行收集工具、组装基础提示词、
//! 直接路由 `model/chat`。Agent 插件与其它插件一样，只通过
//! `traverse(TRAVERSE_AVAILABLE_TOOLS)` 向会话贡献工具。
//!
//! 本 handler 现在**只服务于 `agent_run` 能力派生出的子智能体会话**——
//! 即在一次会话内部，把任务委托给另一个智能体去跑一条独立的会话流。
//!
//! ## 本 handler 保留什么
//!
//! - 校验目标智能体确实存在（不存在就明确报错，绝不静默用默认智能体兜底）
//! - 用**与 session 完全相同**的 `collect_capabilities` 管线装配工具集
//!   （这样子会话拿到的工具与顶层会话是同一套机制、同一套结果）
//! - 转交 `model/chat`
//!
//! ## 本 handler 不再做什么
//!
//! - ❌ 构建系统提示词：人格改由 `agent_identity` 能力的**工具说明**承载
//! - ❌ 每轮注入 `<active_memory>` / `<task_context>`：工作记忆不再自动灌入，
//!   由模型按需调用 `agent_cognition`（`memory.retrieve`）主动回忆

use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::schemas::model::model_chat;
use crate::symbio_core::{
    collect_capabilities, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError,
    PluginPayload, MODEL_CHAT,
};
use std::sync::Arc;

pub async fn handle(
    plugin: &AgentPlugin,
    ctx: Arc<dyn InvokeRequest>,
    workdir_opt: Option<&str>,
) -> InvokeResponse<PluginPayload> {
    let req: model_chat::Request = ctx.payload()?;

    let agent_id = ctx
        .get(crate::symbio_core::AGENT_ID)
        .ok_or_else(|| PluginError::ValidationError("Missing agent_id".to_string()))?;

    // 智能体解析失败必须显式报错——绝不能静默发出"无人格"的会话，
    // 否则模型失去身份约束会凭空臆造（表现为「agent_id 没生效」）。
    if plugin
        .get_mindscape(workdir_opt, &agent_id)
        .await
        .is_none()
    {
        return Err(PluginError::NotFound(format!(
            "Agent '{agent_id}' not found (workdir={workdir_opt:?}); 无法在不解析到具体智能体的情况下开始对话。"
        )));
    }

    // ── 工具装配：与顶层会话同一条管线（symbio_core::collect_capabilities）──
    let parent = plugin.get_parent().await;
    let tool_manager = collect_capabilities(parent.as_ref(), &ctx).await;

    // 收集期若出现硬错误（例如嵌套更深处的插件报了致命问题），直接中止子会话
    let errors = crate::symbio_core::take_errors(&ctx).await;
    if let Some(first) = errors.into_iter().next() {
        return Err(PluginError::NotFound(first.message));
    }

    crate::symbio_core::attach_capabilities(&ctx, tool_manager);

    // ── 路由到 MODEL 服务 ──
    if let Some(parent) = plugin.get_parent().await {
        let final_ctx = ctx.fork();
        final_ctx.set(crate::symbio_core::PATH, MODEL_CHAT.to_string());
        // 显式 set WORKDIR 给下游 model/chat 路由链
        // （fork 虽会继承父 ctx 的 WORKDIR，但显式 set 能让"子 agent 工作目录覆盖父 agent"语义无歧义，
        //  同时保护子智能体内部所有 local 工具的沙箱边界）
        if let Some(wd) = workdir_opt {
            final_ctx.set(crate::symbio_core::WORKDIR, wd.to_string());
        }
        final_ctx.set_payload(req)?;
        return parent.route(final_ctx).await;
    }
    Err(PluginError::InternalError("父插件未设置".to_string()))
}
