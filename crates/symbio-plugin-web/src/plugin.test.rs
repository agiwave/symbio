//! `crates/symbio-plugin-web/src/plugin.rs` 的单元测试 —— 与实现同级分文件。
//!
//! 锁的是「工具没有第二条执行面」（`D-466`）：工具执行只发生在 `port.tool` 的逐调用
//! 簿记里，本插件的 `route` 只剩父容器转发与 `NotFound`。

use super::*;
use symbio_plugin_api::{PluginSimpleRequest, PATH};

/// 按工具名走 `route` 一律 `NotFound`，不执行工具。
///
/// `http_request` 是本插件里声明 `High` 风险的工具：拿它当反例比拿一个无风险工具
/// 更能钉住「闸门只有一个调用点」这件事——高风险工具也无法绕开 `GatedExecutor`。
#[tokio::test]
async fn route_does_not_execute_tools_by_name() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-web-plugin-route"),
        PLUGIN_ID_WEB,
    );
    let plugin: Arc<dyn Plugin> = Arc::new(WebPlugin::new(None, WebConfig::default(), dir));
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(PluginSimpleRequest::from_parts(serde_json::json!({}), None));
    ctx.set(PATH, "http_request".to_string());

    let resp = plugin.route(ctx).await;

    assert!(
        matches!(resp, Err(PluginError::NotFound(_))),
        "按工具名 route 必须 NotFound（工具只经端口执行）"
    );
}

/// 搜索注册事实必须来自当前配置的实际 endpoint，且不能夹带凭据。
#[test]
fn web_search_registration_publishes_configured_upstream_url_fact() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-web-plugin-search-facts"),
        PLUGIN_ID_WEB,
    );
    let plugin = WebPlugin::new(
        None,
        WebConfig {
            tavily_api_key: Some("never-publish-this-key".to_string()),
            search_api_base: "https://proxy.example/base/".to_string(),
            ..WebConfig::default()
        },
        dir,
    );
    let entry = plugin
        .tool_impls
        .iter()
        .find(|entry| entry.meta.name == "web_search")
        .unwrap();
    let facts_of = entry.facts.as_ref().unwrap();
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(PluginSimpleRequest::from_parts(serde_json::json!({}), None));

    let facts = facts_of(&serde_json::json!({"query": "rust"}), &ctx);

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].kind, "url");
    assert_eq!(facts[0].value, "https://proxy.example/base/search");
    assert!(!facts[0].value.contains("never-publish-this-key"));

    {
        let mut cfg = plugin.config.blocking_write();
        cfg.search_api_base = "https://current.example/".to_string();
    }
    let facts = facts_of(&serde_json::json!({"query": "rust"}), &ctx);
    assert_eq!(facts[0].value, "https://current.example/search");

    // 同步事实回调在写锁窗口也必须可用：退稳定官方 endpoint，不 panic、不读 key。
    let fallback = {
        let _cfg = plugin.config.blocking_write();
        facts_of(&serde_json::json!({"query": "rust"}), &ctx)
    };
    assert_eq!(
        fallback[0].value,
        "https://html.duckduckgo.com/html/?q=rust"
    );
}

#[tokio::test]
async fn search_facts_reread_config_written_by_another_process() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-web-plugin-search-refresh"),
        PLUGIN_ID_WEB,
    );
    let plugin = WebPlugin::new(None, WebConfig::default(), dir.clone());
    let fresh = WebConfig {
        search_api_base: "https://fresh.example/base/".to_string(),
        tavily_api_key: Some("fresh-key".to_string()),
        ..WebConfig::default()
    };
    std::fs::create_dir_all(dir.dir()).expect("插件目录");
    std::fs::write(
        dir.config_path(),
        serde_json::to_string(&fresh).expect("配置可序列化"),
    )
    .expect("写入磁盘配置");

    plugin.refresh_config().await;

    let entry = plugin
        .tool_impls
        .iter()
        .find(|entry| entry.meta.name == "web_search")
        .expect("web_search 工具");
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(PluginSimpleRequest::from_parts(serde_json::json!({}), None));
    let facts =
        entry.facts.as_ref().expect("事实回调")(&serde_json::json!({"query": "rust"}), &ctx);
    assert_eq!(facts[0].value, "https://fresh.example/base/search");
}
