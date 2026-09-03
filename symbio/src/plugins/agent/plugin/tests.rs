//! agent plugin 单元测试
//!
//! 对应源文件: `plugin.rs`

use super::*;
use crate::symbio_core::SimpleRequest;
use std::sync::Arc;

fn make_plugin() -> Arc<AgentPlugin> {
    Arc::new(AgentPlugin::new(None, AgentConfig::default()))
}

fn make_ctx(path: &str, workdir: Option<&str>) -> Arc<dyn InvokeRequest> {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set(crate::symbio_core::PATH, path.to_string());
    if let Some(wd) = workdir {
        ctx.set(crate::symbio_core::WORKDIR, wd.to_string());
    }
    ctx
}

#[tokio::test]
async fn test_route_unknown_path_returns_not_found() {
    let plugin = make_plugin();
    let ctx = make_ctx("nonsense", None);
    let result = plugin.route(ctx).await;
    assert!(matches!(result, Err(PluginError::NotFound(_))));
}

/// 旧 `list` 路径在统一资源协议重构后并入 `resources/list`，
/// 本用例改为校验带前导斜杠的**真实当前路由**能被正确剥离并分发。
#[tokio::test]
async fn test_route_strips_leading_slash() {
    let plugin = make_plugin();
    let ctx = make_ctx("/resources/list", None);
    let res = plugin.route(ctx).await;
    // 成功返回 Data / Empty / Session / 任意非错误；或路由内参数校验错误
    assert!(res.is_ok() || matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_traverse_wrong_path_returns_not_found() {
    let plugin = make_plugin();
    let ctx = make_ctx("not_tools", None);
    let result = plugin.traverse("".to_string(), ctx).await;
    assert!(matches!(result, Err(PluginError::NotFound(_))));
}

/// 未选择智能体（ctx 无 AGENT_ID）→ traverse 静默成功且**不注册任何工具**。
///
/// 这是"agent 降级为普通插件"重构的核心契约：不选 agent 的会话照常运行。
#[tokio::test]
async fn test_traverse_without_agent_id_registers_nothing() {
    use crate::symbio_core::CapabilityManager;

    let plugin = make_plugin();
    let ctx = make_ctx(crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS, None);
    // 不设置 AGENT_ID —— 模拟"未选择智能体"的会话
    let tool_manager: Arc<dyn CapabilityManager> =
        Arc::new(crate::symbio_core::DefaultToolManager::new());
    ctx.set(crate::symbio_core::CAPABILITY_MANAGER, tool_manager.clone());

    let result = plugin.traverse("".to_string(), ctx).await;
    assert!(result.is_ok(), "无 AGENT_ID 时 traverse 应静默成功");

    let caps = tool_manager.list_capability().await;
    assert!(
        caps.is_empty(),
        "无 AGENT_ID 时不应注册任何工具，实际注册了: {:?}",
        caps.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
    );
}

/// 选定了不存在的智能体 → traverse 报告收集期硬错误（写入 ctx 错误桶），
/// 且不注册工具。session 编排方据此中止会话并明确报错。
#[tokio::test]
async fn test_traverse_with_unknown_agent_reports_error() {
    use crate::symbio_core::CapabilityManager;

    let plugin = make_plugin();
    let ctx = make_ctx(crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS, None);
    ctx.set(crate::symbio_core::AGENT_ID, "ghost_agent".to_string());
    let tool_manager: Arc<dyn CapabilityManager> =
        Arc::new(crate::symbio_core::DefaultToolManager::new());
    ctx.set(crate::symbio_core::CAPABILITY_MANAGER, tool_manager.clone());

    let result = plugin.traverse("".to_string(), ctx.clone()).await;
    assert!(result.is_ok(), "traverse 本身不应失败（错误经桶传递）");

    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(
        !errors.is_empty(),
        "智能体不存在时应报告收集期硬错误"
    );

    let caps = tool_manager.list_capability().await;
    assert!(caps.is_empty(), "智能体不存在时不应注册任何工具");
}

#[tokio::test]
async fn test_resolve_mindscape_missing_agent_id() {
    let plugin = make_plugin();
    let ctx = Arc::new(SimpleRequest::new(None, None));
    let result = plugin.resolve_mindscape_from_ctx(ctx.as_ref()).await;
    assert!(matches!(result, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_resolve_mindscape_missing_agent_returns_not_found() {
    let plugin = make_plugin();
    let ctx = Arc::new(SimpleRequest::new(None, None));
    ctx.set(
        crate::symbio_core::AGENT_ID,
        "nonexistent_agent".to_string(),
    );
    let result = plugin.resolve_mindscape_from_ctx(ctx.as_ref()).await;
    assert!(matches!(result, Err(PluginError::NotFound(_))));
}

#[test]
fn test_metadata_id() {
    let m = AgentPlugin::metadata();
    assert_eq!(m.id, "agent");
    assert_eq!(m.name, "智能体与心智流形");
}

#[test]
fn test_default_config_works() {
    let _plugin = AgentPlugin::new(None, AgentConfig::default());
}

#[test]
fn test_payload_set_and_get() {
    let ctx = Arc::new(SimpleRequest::new(None, None));
    let req: Result<serde_json::Value, _> = ctx.payload();
    assert!(req.is_err(), "空 ctx 应当无 payload");
    ctx.set_payload(serde_json::json!({"test": 1})).unwrap();
    let got: serde_json::Value = ctx.payload().unwrap();
    assert_eq!(got["test"], 1);
}

#[test]
fn test_agent_capability_registrations() {
    // 锁定 Agent 插件声明装载的 capability 数量，防止意外增减破坏工具列表
    // 4 个能力: identity（人格载体）, chat（子智能体委托）, cognition（统一认知）, create_agent
    //
    // 验证策略：
    // 1. 本模块的 id 数组（来自 symbio_core 常量） == 4 个
    // 2. 数组中每个 id 都已经在系统中通过 submit_object_creator! 注册过
    //    （且注册侧也使用了同一份 symbio_core 常量，杜绝拼写漂移）
    assert_eq!(AGENT_CAPABILITY_IDS.len(), 4, "期望 4 个 capability id");

    // 同时校验：每个 id 都已经在系统中通过 submit_object_creator! 注册过
    for id in AGENT_CAPABILITY_IDS {
        assert!(
            crate::symbio_core::has_creator(id),
            "capability `{}` 缺少 submit_object_creator! 注册",
            id
        );
    }

    // 4 个能力常量必须互不相同
    let names: Vec<&str> = AGENT_CAPABILITY_IDS.to_vec();
    let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), 4, "AGENT_CAPABILITY_IDS 内部出现重复 id");
}
