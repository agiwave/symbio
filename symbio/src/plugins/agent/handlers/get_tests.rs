//! agent get handler 单元测试
//!
//! 对应源文件: `get.rs`

use super::*;
use crate::plugins::agent::core::AgentConfig;
use crate::symbio_core::SimpleRequest;
use serde_json::json;

fn make_plugin() -> Arc<AgentPlugin> {
    Arc::new(AgentPlugin::new(None, AgentConfig::default()))
}

fn make_ctx_with_id(id: &str) -> Arc<dyn InvokeRequest> {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set(crate::symbio_core::AGENT_ID, id.to_string());
    ctx
}

fn make_ctx_with_payload_id(id: &str) -> Arc<dyn InvokeRequest> {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set_payload(json!({"id": id})).unwrap();
    ctx
}

#[tokio::test]
async fn test_get_missing_id_returns_validation_error() {
    let plugin = make_plugin();
    let ctx = Arc::new(SimpleRequest::new(None, None));
    let res = handle(&plugin, ctx, None).await;
    assert!(matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_get_unknown_agent_returns_not_found() {
    let plugin = make_plugin();
    let ctx = make_ctx_with_id("nonexistent_agent_xyz");
    let res = handle(&plugin, ctx, None).await;
    assert!(matches!(res, Err(PluginError::NotFound(_))));
}

#[tokio::test]
async fn test_get_from_payload_id() {
    let plugin = make_plugin();
    let ctx = make_ctx_with_payload_id("nonexistent_xyz");
    let res = handle(&plugin, ctx, None).await;
    // 走 payload 路径应也能识别
    assert!(matches!(res, Err(PluginError::NotFound(_))));
}
