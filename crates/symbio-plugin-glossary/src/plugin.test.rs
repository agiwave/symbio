//! 插件身份与工具元数据的判据。

use super::{GlossaryPlugin, GlossaryTool};
use crate::behavior::GLOSSARY_RECORD;
use symbio_plugin_api::CapabilityCategory;
use symbio_plugin_api::PLUGIN_ID_GLOSSARY;

#[test]
fn the_plugin_identity_comes_from_the_manifest() {
    let meta = GlossaryPlugin::metadata();
    assert_eq!(meta.id, PLUGIN_ID_GLOSSARY);
    assert_eq!(meta.name, "术语表");
}

/// 工具在元数据里声明了它归属的行为名——宿主据此选效果实现，不认工具名。
#[test]
fn the_tool_declares_its_behavior() {
    let meta = GlossaryTool.meta();
    assert_eq!(meta.name, "glossary_define");
    assert!(meta.declares(GLOSSARY_RECORD));
    assert_eq!(meta.category, Some(CapabilityCategory::Other));
}

/// `meta.declares` 是元数据自有的判据（不再经由已退场的 `Capability` trait）。
#[test]
fn the_meta_declares_directly() {
    let meta = GlossaryTool.meta();
    assert!(!meta.declares("glossary_other"));
}
