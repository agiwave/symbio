//! `symbio/src/plugins/local/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 工具清单回归护栏。
///
/// `codebase_search` 曾被临时摘出 `tool_impls`（提交 `30ef62c`）。摘掉之后语义检索
/// 直接消失，而其余单测全绿——只有显式断言工具清单才拦得住这种静默退化。
#[test]
fn codebase_search_is_registered() {
    let dir = PluginDir::at(
        std::env::temp_dir().join("symbio-local-plugin-tool-list"),
        PLUGIN_LOCAL,
    );
    let plugin = LocalPlugin::new(None, LocalConfig::default(), dir);
    let names: Vec<String> = plugin.tool_impls.iter().map(|t| t.name()).collect();
    assert!(
        names.iter().any(|n| n == "codebase_search"),
        "codebase_search 未注册，实际工具清单：{names:?}"
    );
}
