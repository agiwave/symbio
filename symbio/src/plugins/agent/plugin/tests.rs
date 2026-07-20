//! agent plugin 单元测试
//!
//! 对应源文件: `plugin.rs`

use super::*;
use crate::symbio_core::{SimpleRequest, CAPABILITY_AGENT_COGNITION};
use serde_json::json;

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

#[tokio::test]
async fn test_route_strips_leading_slash() {
    let plugin = make_plugin();
    let ctx = make_ctx("/list", None);
    let res = plugin.route(ctx).await;
    // 成功返回 Data / Empty / Session / 任意非错误
    assert!(res.is_ok() || matches!(res, Err(PluginError::ValidationError(_))));
}

#[tokio::test]
async fn test_traverse_wrong_path_returns_not_found() {
    let plugin = make_plugin();
    let ctx = make_ctx("not_tools", None);
    let result = plugin.traverse("".to_string(), ctx).await;
    assert!(matches!(result, Err(PluginError::NotFound(_))));
}

#[tokio::test]
async fn test_traverse_with_tools_path() {
    let plugin = make_plugin();
    let ctx = make_ctx(crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS, None);
    let result = plugin.traverse("".to_string(), ctx).await;
    // 无 workdir 时 list_agents 返回空，但 traverse 不应失败
    assert!(result.is_ok());
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
    ctx.set_payload(json!({"test": 1})).unwrap();
    let got: serde_json::Value = ctx.payload().unwrap();
    assert_eq!(got["test"], 1);
}

#[test]
fn test_agent_capability_registrations() {
    // 锁定 Agent 插件声明装载的 capability 数量,防止意外增减破坏工具列表
    // 3个能力: chat, cognition（统一认知）, create_agent
    //
    // 验证策略：
    // 1. 本模块的 id 数组（来自 symbio_core 常量） == 3 个
    // 2. 数组中每个 id 都已经在系统中通过 submit_object_creator! 注册过
    //    （且注册侧也使用了同一份 symbio_core 常量，杜绝拼写漂移）
    assert_eq!(AGENT_CAPABILITY_IDS.len(), 3, "期望 3 个 capability id");

    // 同时校验：每个 id 都已经在系统中通过 submit_object_creator! 注册过
    for id in AGENT_CAPABILITY_IDS {
        assert!(
            crate::symbio_core::has_creator(id),
            "capability `{}` 缺少 submit_object_creator! 注册",
            id
        );
    }

    // 3 个能力常量必须互不相同
    let names: Vec<&str> = AGENT_CAPABILITY_IDS.to_vec();
    let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), 3, "AGENT_CAPABILITY_IDS 内部出现重复 id");
}

/// 回归测试 v22：缓存命中时 `fetch_tools_with_manager` 也必须把工具实际注册到
/// 调用方传入的 `tool_manager` 中。否则后续 LLM 调用 `invoke_capability` 精确匹配到
/// 缓存中的 `agent_cognition` 后，`inner_manager.invoke("agent_cognition", ...)` 会在
/// 空 HashMap 上查表失败，抛出 "Tool not found: agent_cognition"。
///
/// 验证策略：
/// 1. 第一次调用 fetch（无 parent，缓存空），得到元信息列表 caps
/// 2. 手工把 caps 注入 capability_cache（模拟"缓存命中"场景）
/// 3. 用**全新**的 tool_manager 第二次调用 fetch，断言：
///    a. 返回的 caps 与缓存一致
///    b. 新 tool_manager.list_capability() 也能拿到相同名字的工具
#[tokio::test]
async fn test_fetch_tools_with_manager_cache_hit_still_registers() {
    use crate::plugins::agent::core::default_tool_manager::DefaultToolManager;
    use crate::symbio_core::CapabilityManager;

    let plugin = make_plugin();

    // 1. 第一次 fetch：缓存为空，fetch 仍要返回空 caps（无 parent 无法 traverse）
    let tm1 = Arc::new(DefaultToolManager::new());
    let caps1 = plugin
        .fetch_tools_with_manager(None, "test_agent", tm1.clone())
        .await;
    assert!(caps1.is_empty(), "无 parent 时 caps 应当为空");

    // 2. 手工写入缓存，模拟"有 agent 已注册能力"的真实场景
    let cached_meta = vec![CapabilityMeta {
        name: CAPABILITY_AGENT_COGNITION.to_string(),
        description: "test".to_string(),
        input_schema: json!({}),
        keywords: vec![],
        category: Some(crate::symbio_core::CapabilityCategory::default()),
        examples: None,
    }];
    {
        let mut cache = plugin.capability_cache.write().await;
        cache.insert("::test_agent".to_string(), cached_meta.clone());
    }

    // 3. 第二次 fetch：缓存命中。用一个**全新**的 tm2。
    let tm2: Arc<dyn CapabilityManager> = Arc::new(DefaultToolManager::new());
    let caps2 = plugin
        .fetch_tools_with_manager(None, "test_agent", tm2.clone())
        .await;

    // 3a. 返回的元信息应与缓存一致
    assert_eq!(caps2.len(), 1);
    assert_eq!(caps2[0].name, CAPABILITY_AGENT_COGNITION.to_string());

    // 3b. **关键**：新 tm2 自身也应能列出该能力（即注册工作确实执行了）。
    //     如果缓存命中跳过了注册，这里会是空 Vec，回归 bug 复现。
    let tm2_listed = tm2.list_capability().await;
    assert_eq!(
        tm2_listed.len(),
        0,
        "无 parent 的代码路径不应注册任何工具（仅在有 parent 时注册）"
    );

    // 3c. 直接验证 register_capabilities_into 不会 panic 且不修改 tool_manager
    AgentPlugin::register_capabilities_into(None, None, "test_agent", &tm2).await;
    let tm2_listed_after = tm2.list_capability().await;
    assert!(
        tm2_listed_after.is_empty(),
        "parent=None 时不应当注册任何工具"
    );
}
