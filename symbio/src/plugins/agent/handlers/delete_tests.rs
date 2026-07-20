//! agent delete handler 单元测试
//!
//! 对应源文件: `delete.rs`

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

#[tokio::test]
async fn test_delete_missing_id_returns_validation_error() {
    let plugin = make_plugin();
    let ctx = Arc::new(SimpleRequest::new(None, None));
    let res = handle(&plugin, ctx, None).await;
    assert!(matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_delete_nonexistent_returns_idempotent_false() {
    let plugin = make_plugin();
    let ctx = make_ctx_with_id("nonexistent_agent_xyz");
    let res = handle(&plugin, ctx, None).await;
    assert!(res.is_ok(), "缺失 agent 应当幂等返回 deleted=false");
    let payload: serde_json::Value = res.unwrap().get().unwrap_or(json!({}));
    assert_eq!(payload["deleted"], false);
}

#[tokio::test]
async fn test_delete_from_payload_id() {
    let plugin = make_plugin();
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set_payload(json!({"id": "payload_id_agent"})).unwrap();
    let res = handle(&plugin, ctx, None).await;
    // 不存在时仍返回 deleted=false（幂等）
    assert!(res.is_ok());
}
