//! `crates/symbio-plugin-local/src/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use symbio_plugin_api::{PluginSimpleRequest, PATH};

/// 工具清单回归护栏。
///
/// `codebase_search` 曾被临时摘出 `tool_impls`（提交 `30ef62c`）。摘掉之后语义检索
/// 直接消失，而其余单测全绿——只有显式断言工具清单才拦得住这种静默退化。
#[test]
fn codebase_search_is_registered() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-local-plugin-tool-list"),
        PLUGIN_ID_LOCAL,
    );
    let plugin = LocalPlugin::new(None, LocalConfig::default(), dir);
    let names: Vec<String> = plugin
        .tool_impls
        .iter()
        .map(|t| t.meta.name.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "codebase_search"),
        "codebase_search 未注册，实际工具清单：{names:?}"
    );
}

/// 工具没有第二条执行面（`D-466`）：按工具名走 `route` 一律 `NotFound`。
///
/// 「判闸 + 造 `PortExecContext` + 执行 + 拆信封」只发生在 `port.tool` 的逐调用簿记里；
/// 本插件的 `route` 只剩父容器转发与 `NotFound`。有人把工具执行接回 `route`，
/// 这条用例就是那道绊线。
#[tokio::test]
async fn route_does_not_execute_tools_by_name() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-local-plugin-route"),
        PLUGIN_ID_LOCAL,
    );
    let plugin: Arc<dyn Plugin> = Arc::new(LocalPlugin::new(None, LocalConfig::default(), dir));
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(PluginSimpleRequest::from_parts(json!({}), None));
    ctx.set(PATH, "todo_write".to_string());

    let resp = plugin.route(ctx).await;

    assert!(
        matches!(resp, Err(PluginError::NotFound(_))),
        "按工具名 route 必须 NotFound（工具只经端口执行）"
    );
}
