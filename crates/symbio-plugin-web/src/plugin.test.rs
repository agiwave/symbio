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
