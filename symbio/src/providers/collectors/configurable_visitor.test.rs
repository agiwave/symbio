//! `providers/collectors/configurable_visitor.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//! 这两条原在 `symbio_core/capability/configurable.test.rs`，随默认收集器一起迁出 core；
//! 同文件里测 `capability_entry_of` 的那条是**契约**，留在 core。

use super::*;

use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::{capability_entry_of, PluginConfigFile, PluginDir};

fn config(dir_name: &str, title: &str) -> PluginConfigFile {
    PluginConfigFile::new(
        PluginDir::at(std::env::temp_dir(), dir_name),
        title,
        DetailDefinition::default(),
    )
}

/// 同目录名覆盖，槽位不变（IndexMap 保序）
#[tokio::test]
async fn register_keeps_order_and_overwrites_same_name() {
    let v = DefaultConfigurableVisitor::new();
    v.register_configurable(capability_entry_of(&config("web", "网络工具")))
        .await;
    v.register_configurable(capability_entry_of(&config("session", "会话设置")))
        .await;
    v.register_configurable(capability_entry_of(&config("web", "网络工具（改）")))
        .await;

    let list = v.list_configurables().await;
    assert_eq!(
        list.iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["web", "session"]
    );
    assert_eq!(list[0].node.title, "网络工具（改）");
}

#[tokio::test]
async fn empty_visitor_lists_nothing() {
    let v = DefaultConfigurableVisitor::new();
    assert!(v.list_configurables().await.is_empty());
}
