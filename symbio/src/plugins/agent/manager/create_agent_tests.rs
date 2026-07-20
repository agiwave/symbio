//! AgentCreateTool 单元测试
//!
//! 对应源文件: `create_agent.rs`

use super::*;
use crate::plugins::agent::core::AgentConfig;
use crate::symbio_core::SimpleRequest;

fn make_plugin() -> Arc<AgentPlugin> {
    Arc::new(AgentPlugin::new(None, AgentConfig::default()))
}

fn make_ctx_with_payload(payload: serde_json::Value) -> Arc<dyn InvokeRequest> {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set_payload(payload).unwrap();
    ctx
}

#[tokio::test]
async fn test_help_request_returns_documentation() {
    let plugin = make_plugin();
    let tool = AgentCreateTool::new(plugin);
    let ctx = make_ctx_with_payload(json!({"help": true}));
    let res = tool.execute(ctx).await;
    assert!(res.is_ok(), "help 请求应成功");
}

#[tokio::test]
async fn test_missing_id_returns_validation_error() {
    let plugin = make_plugin();
    let tool = AgentCreateTool::new(plugin);
    let ctx = make_ctx_with_payload(json!({"cognition_units": []}));
    let res = tool.execute(ctx).await;
    assert!(matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_empty_id_returns_validation_error() {
    let plugin = make_plugin();
    let tool = AgentCreateTool::new(plugin);
    let ctx = make_ctx_with_payload(json!({"id": "  ", "cognition_units": []}));
    let res = tool.execute(ctx).await;
    assert!(matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_missing_cognition_units_returns_validation_error() {
    let plugin = make_plugin();
    let tool = AgentCreateTool::new(plugin);
    let ctx = make_ctx_with_payload(json!({"id": "test"}));
    let res = tool.execute(ctx).await;
    assert!(matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_meta_includes_required_fields() {
    let plugin = make_plugin();
    let tool = AgentCreateTool::new(plugin);
    let meta = tool.meta();
    assert_eq!(meta.name, CAPABILITY_AGENT_CREATE.to_string());
    let schema = meta.input_schema;
    assert!(schema.get("properties").is_some());
    assert!(schema["properties"]["id"].is_object());
    assert!(schema["properties"]["cognition_units"].is_object());
}
