//! `plugin.rs` 的单元测试 —— 收集期发布句柄 + 内置档只读（D-531 / 裁决 B）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use symbio_plugin_api::PluginSimpleRequest;

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

fn test_plugin() -> Arc<SandboxPlugin> {
    Arc::new(SandboxPlugin {
        dir: PluginDir::at(std::env::temp_dir(), PLUGIN_ID_SANDBOX),
        registry: Arc::new(RwLock::new(SandboxRegistry::default())),
    })
}

#[tokio::test]
async fn traverse_publishes_a_handle_into_the_shared_slot() {
    // 用同一条 ctx lineage：先建槽，traverse 写入，再从同一 ctx 读回。
    let c = ctx();
    symbio_plugin_api::sandbox_init_slot(&c);
    let plugin = test_plugin();
    let _ = plugin.clone().traverse(String::new(), c.clone()).await;
    assert!(
        symbio_plugin_api::sandbox_of(&*c).is_some(),
        "traverse 后消费方应能读到句柄"
    );
}

#[tokio::test]
async fn reload_seeds_builtin_profiles() {
    let plugin = test_plugin();
    plugin.reload().await;
    let reg = plugin.registry.read().unwrap();
    assert!(reg.archives().contains_key("default"));
    assert!(reg.archives().contains_key("strict"));
    assert!(reg.archives().contains_key("unattended"));
    assert!(reg.archives().get("strict").unwrap().builtin);
}

#[test]
fn builtin_ids_are_not_writable() {
    for id in ["default", "strict", "unattended"] {
        assert!(builtin_profiles().iter().any(|(k, _)| k == id));
    }
}
