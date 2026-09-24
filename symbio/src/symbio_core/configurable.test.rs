//! `symbio/src/symbio_core/configurable.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::plugin_dir::PluginDir;
use crate::symbio_core::schemas::detail::DetailDefinition;

fn config() -> ConfigFile {
    ConfigFile::new(
        PluginDir::at(std::env::temp_dir(), "web"),
        "网络工具",
        DetailDefinition::default(),
    )
}

/// 条目 = 配置文档的节点视图，但**名字与地址按列表口径**改掉
#[test]
fn entry_keeps_the_document_view_but_relabels_it() {
    let n = entry_of(&config());

    // 地址 = 真实地址（读写仍落在拥有者自己的文件上）
    assert_eq!(n.path, "web/PLUGIN.yml");
    // 名字 = 目录名：列表内唯一，也是前端查图标的键
    assert_eq!(n.name, "web");
    // 标题与呈现定义来自配置文档自己
    assert_eq!(n.title, "网络工具");
    assert_eq!(n.ext.as_deref(), Some("form"));
    assert!(n.schema.is_some());
    assert!(n.access.read && n.access.write);
}

/// 同目录名覆盖，槽位不变（IndexMap 保序）
#[tokio::test]
async fn register_keeps_order_and_overwrites_same_name() {
    let v = DefaultConfigurableVisitor::new();
    v.register_configurable(entry_of(&config())).await;
    v.register_configurable(entry_of(&ConfigFile::new(
        PluginDir::at(std::env::temp_dir(), "session"),
        "会话设置",
        DetailDefinition::default(),
    )))
    .await;
    v.register_configurable(entry_of(&ConfigFile::new(
        PluginDir::at(std::env::temp_dir(), "web"),
        "网络工具（改）",
        DetailDefinition::default(),
    )))
    .await;

    let list = v.list_configurables().await;
    assert_eq!(
        list.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        vec!["web", "session"]
    );
    assert_eq!(list[0].title, "网络工具（改）");
}

#[tokio::test]
async fn empty_visitor_lists_nothing() {
    let v = DefaultConfigurableVisitor::new();
    assert!(v.list_configurables().await.is_empty());
}
