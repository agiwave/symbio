//! agent chat handler（子智能体会话执行入口）单元测试
//!
//! 对应源文件: `chat.rs`
//!
//! 重构后本 handler 只服务于 `agent_run` 派生的子智能体会话：
//! 校验智能体存在 → 统一管线收集工具 → 转交 model/chat。

use super::*;
use crate::symbio_core::SimpleRequest;
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginError};
use std::sync::Arc;

fn make_plugin() -> &'static crate::plugins::agent::plugin::AgentPlugin {
    // 泄漏一个小插件供测试用（单测进程生命周期内安全，避免静态引用的笨重替代写法）
    Box::leak(Box::new(crate::plugins::agent::plugin::AgentPlugin::new(
        None,
        Default::default(),
    )))
}

fn make_chat_ctx(agent_id: Option<&str>) -> Arc<dyn InvokeRequest> {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    if let Some(aid) = agent_id {
        ctx.set(crate::symbio_core::AGENT_ID, aid.to_string());
    }
    let req = model_chat::Request::default();
    ctx.set_payload(req).expect("set payload failed");
    ctx
}

/// 缺少 agent_id → 明确的 ValidationError（子智能体会话必须知道委托给谁）
#[tokio::test]
async fn missing_agent_id_returns_validation_error() {
    let plugin = make_plugin();
    let ctx = make_chat_ctx(None);
    let result = handle(plugin, ctx, None).await;
    assert!(
        matches!(result, Err(PluginError::ValidationError(_))),
        "缺 agent_id 应返回 ValidationError，实际: {:?}",
        result.err()
    );
}

/// 智能体不存在 → NotFound，绝不静默发出"无人格"的子会话
#[tokio::test]
async fn unknown_agent_returns_not_found() {
    let plugin = make_plugin();
    let ctx = make_chat_ctx(Some("ghost_agent"));
    let result = handle(plugin, ctx, None).await;
    assert!(
        matches!(result, Err(PluginError::NotFound(ref e)) if e.contains("ghost_agent")),
        "不存在的智能体应返回 NotFound（含智能体名），实际: {:?}",
        result.err()
    );
}
