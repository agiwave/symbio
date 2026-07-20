//! McpManager 单元测试
//!
//! 对应源文件: `manager.rs`

use super::*;
use crate::plugins::mcp::types::McpTool;
use crate::symbio_core::schemas::mcp::mcp_config::{McpServerConfig, McpTransportType};
use serde_json::json;

/// TEST-M2.1：stdio transport 但缺 command 字段 → 返回错误
#[tokio::test]
async fn discover_tools_stdio_missing_command() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: None,
        ..Default::default()
    };
    let err = mgr.discover_tools_stdio(&cfg).await.unwrap_err();
    assert!(err.contains("command"), "unexpected error: {err}");
}

/// TEST-M2.2：http transport 但缺 url 字段 → 返回错误
#[tokio::test]
async fn discover_tools_http_missing_url() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Http,
        url: None,
        ..Default::default()
    };
    let err = mgr.discover_tools_http("test", &cfg).await.unwrap_err();
    assert!(err.contains("url"), "unexpected error: {err}");
}

/// TEST-M3.1：stdio call_tool 缺 command → 返回错误
#[tokio::test]
async fn call_tool_stdio_missing_command() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: None,
        ..Default::default()
    };
    let err = mgr
        .call_tool_stdio(&cfg, "tool", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("command"), "unexpected error: {err}");
}

/// TEST-M8.1：McpManager::new 持有共享的 http_client
#[test]
fn mcp_manager_holds_http_client() {
    let mgr = McpManager::new();
    // http_client 字段存在即视为共享 client 持有
    let _ = mgr.http_client.clone();
}

/// TEST-M11.1：discover 缓存命中
#[tokio::test]
async fn discover_tools_cache_hit() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: Some("__nonexistent__".to_string()),
        ..Default::default()
    };
    // 第一次失败（spawn 失败）写入缓存了吗？— 错误不写缓存
    // 改为预填充缓存
    let mut cache = mgr.tools_cache.lock().await;
    cache.insert(
        "cached_server".to_string(),
        CachedTools {
            tools: vec![McpTool {
                name: "test_tool".to_string(),
                description: "test".to_string(),
                input_schema: json!({}),
                annotations: None,
            }],
            inserted_at: Instant::now(),
        },
    );
    drop(cache);
    // 缓存命中：不会触发实际 transport 调用
    let result = mgr.discover_tools("cached_server", &cfg).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "test_tool");
}

/// TEST-M11.2：invalidate_discover_cache 清除缓存
#[tokio::test]
async fn invalidate_discover_cache_clears_entry() {
    let mgr = McpManager::new();
    let mut cache = mgr.tools_cache.lock().await;
    cache.insert(
        "x".to_string(),
        CachedTools {
            tools: vec![],
            inserted_at: Instant::now(),
        },
    );
    drop(cache);
    mgr.invalidate_discover_cache("x").await;
    let cache = mgr.tools_cache.lock().await;
    assert!(!cache.contains_key("x"));
}

/// TEST-M11.3：per-server Mutex 序列化
#[tokio::test]
async fn per_server_lock_serializes() {
    let mgr = McpManager::new();
    let lock1 = mgr.get_server_lock("srv1").await;
    let lock2 = mgr.get_server_lock("srv1").await;
    assert!(Arc::ptr_eq(&lock1, &lock2), "同一 server 应共享锁");
}

/// TEST-M11.4：不同 server 用不同锁
#[tokio::test]
async fn different_servers_have_different_locks() {
    let mgr = McpManager::new();
    let lock1 = mgr.get_server_lock("srv1").await;
    let lock2 = mgr.get_server_lock("srv2").await;
    assert!(!Arc::ptr_eq(&lock1, &lock2), "不同 server 应独立锁");
}

/// TEST-M11.5：McpSessionCache 基本 CRUD
#[tokio::test]
async fn session_cache_basic_crud() {
    let cache = McpSessionCache::new();
    assert!(cache.get("a").await.is_none());
    cache.insert("a".to_string(), "sid-1".to_string()).await;
    assert_eq!(cache.get("a").await, Some("sid-1".to_string()));
    cache.remove("a").await;
    assert!(cache.get("a").await.is_none());
}

/// TEST: discover 路由到 stdio
#[tokio::test]
async fn discover_tools_routes_to_stdio_for_stdio_config() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: Some("__nonexistent_command_for_test__".to_string()),
        ..Default::default()
    };
    let err = mgr.discover_tools("test", &cfg).await.unwrap_err();
    assert!(
        err.to_lowercase().contains("process")
            || err.to_lowercase().contains("command")
            || err.to_lowercase().contains("start"),
        "expected stdio path error, got: {err}"
    );
}

/// TEST: filter_tools 在 manager 入口的应用
#[test]
fn filter_tools_white_list_narrows_tools() {
    let raw = vec![
        McpTool {
            name: "search".to_string(),
            description: "search".to_string(),
            input_schema: json!({}),
            annotations: None,
        },
        McpTool {
            name: "write".to_string(),
            description: "write".to_string(),
            input_schema: json!({}),
            annotations: None,
        },
    ];
    let include = Some(vec!["search".to_string()]);
    let filtered = super::super::types::filter_tools(raw, &include, &None);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "search");
}

/// TEST-MR20.1：test_connection 走 stdio（不存在的命令）→ 返回错误
#[tokio::test]
async fn test_connection_stdio_missing_command() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: Some("__nonexistent_cmd_for_test__".to_string()),
        ..Default::default()
    };
    let result = mgr.test_connection("test", &cfg).await;
    assert!(result.is_err(), "test_connection 应返回错误");
}

/// TEST-MR20.2：test_connection 走 http（缺 url）→ 返回错误
#[tokio::test]
async fn test_connection_http_missing_url() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Http,
        url: None,
        ..Default::default()
    };
    let result = mgr.test_connection("test", &cfg).await;
    assert!(result.is_err(), "test_connection 应返回错误");
}

/// TEST-MR24.1：discover 失败但有陈旧缓存 → 返回陈旧缓存
#[tokio::test]
async fn discover_tools_falls_back_to_stale_cache_on_failure() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: Some("__nonexistent_for_test__".to_string()),
        ..Default::default()
    };
    // 预填充一个**陈旧**（inserted_at 设到很久之前）缓存
    let stale = CachedTools {
        tools: vec![McpTool {
            name: "stale_tool".to_string(),
            description: "from stale cache".to_string(),
            input_schema: json!({}),
            annotations: None,
        }],
        inserted_at: Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now),
    };
    {
        let mut cache = mgr.tools_cache.lock().await;
        cache.insert("flaky_server".to_string(), stale.clone());
    }
    // stdio discover 会失败（命令不存在），但应返回陈旧缓存
    let result = mgr
        .discover_tools("flaky_server", &cfg)
        .await
        .expect("应有陈旧缓存 fallback");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "stale_tool");
}

/// TEST-MR24.2：discover 失败且无任何缓存 → 返回错误
#[tokio::test]
async fn discover_tools_errors_when_no_cache_and_failure() {
    let mgr = McpManager::new();
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Stdio,
        command: Some("__nonexistent_for_test__".to_string()),
        ..Default::default()
    };
    let result = mgr.discover_tools("no_cache_server", &cfg).await;
    assert!(result.is_err(), "无缓存时失败应返回错误");
}

// ===== BUG-MR29：forget_server 清理所有内部状态 =====

/// TEST-MR29.1：forget_server 清理 discover cache
#[tokio::test]
async fn forget_server_clears_cache() {
    let mgr = McpManager::new();
    // 预填充 cache
    {
        let mut cache = mgr.tools_cache.lock().await;
        cache.insert(
            "ghost".to_string(),
            CachedTools {
                tools: vec![],
                inserted_at: Instant::now(),
            },
        );
    }
    mgr.forget_server("ghost").await;
    let cache = mgr.tools_cache.lock().await;
    assert!(!cache.contains_key("ghost"), "discover cache 应被清理");
}

/// TEST-MR29.2：forget_server 清理 session cache
#[tokio::test]
async fn forget_server_clears_session_cache() {
    let mgr = McpManager::new();
    mgr.session_cache
        .insert("ghost".to_string(), "sid-x".to_string())
        .await;
    assert!(mgr.session_cache.get("ghost").await.is_some());
    mgr.forget_server("ghost").await;
    assert!(
        mgr.session_cache.get("ghost").await.is_none(),
        "session cache 应被清理"
    );
}

/// TEST-MR29.3：forget_server 清理 server_locks
#[tokio::test]
async fn forget_server_clears_lock() {
    let mgr = McpManager::new();
    // 先创建锁
    let _lock = mgr.get_server_lock("ghost").await;
    {
        let locks = mgr.server_locks.lock().await;
        assert!(locks.contains_key("ghost"));
    }
    mgr.forget_server("ghost").await;
    let locks = mgr.server_locks.lock().await;
    assert!(!locks.contains_key("ghost"), "server_locks 应被清理");
}

/// TEST-MR29.4：forget_server 幂等：不存在 server 调用也不报错
#[tokio::test]
async fn forget_server_nonexistent_is_idempotent() {
    let mgr = McpManager::new();
    // 不存在的 server：no-op
    mgr.forget_server("never_existed").await;
}

// ===== BUG-MR30：TestConnectionResult 字段 =====

/// TEST-MR30.1：TestConnectionResult 基本字段
#[test]
fn test_connection_result_fields() {
    use super::TestConnectionResult;
    let r = TestConnectionResult {
        tool_count: 5,
        protocol_version: "2025-06-18".to_string(),
        server_name: Some("test-server".to_string()),
        server_version: Some("1.0.0".to_string()),
        instructions: Some("Use carefully".to_string()),
        elapsed_ms: 123,
    };
    assert_eq!(r.tool_count, 5);
    assert_eq!(r.protocol_version, "2025-06-18");
    assert_eq!(r.server_name.as_deref(), Some("test-server"));
    assert_eq!(r.server_version.as_deref(), Some("1.0.0"));
    assert_eq!(r.instructions.as_deref(), Some("Use carefully"));
    assert_eq!(r.elapsed_ms, 123);
}

/// TEST-MR30.2：TestConnectionResult 字段默认
#[test]
fn test_connection_result_default_construction() {
    use super::TestConnectionResult;
    // 直接构造（无便捷 stdio 构造器）
    let r = TestConnectionResult {
        tool_count: 3,
        protocol_version: "2024-11-05".to_string(),
        server_name: None,
        server_version: None,
        instructions: None,
        elapsed_ms: 0,
    };
    assert_eq!(r.tool_count, 3);
    assert_eq!(r.protocol_version, "2024-11-05");
    assert_eq!(r.server_name, None);
    assert_eq!(r.server_version, None);
    assert_eq!(r.instructions, None);
    assert_eq!(r.elapsed_ms, 0);
}
