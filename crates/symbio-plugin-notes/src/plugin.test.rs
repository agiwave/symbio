//! 插件身份与工具元数据的判据。

use super::{NotesPlugin, NotesTool};
use symbio_core::{Capability, CapabilityCategory};
use symbio_deploy::PLUGIN_ID_NOTES;
use symbio_plugin_api::behaviors;

#[test]
fn the_plugin_identity_comes_from_the_manifest() {
    let meta = NotesPlugin::metadata();
    assert_eq!(meta.id, PLUGIN_ID_NOTES);
    assert_eq!(meta.name, "便签");
}

/// 工具在元数据里声明了它归属的行为名——宿主据此选效果实现，不认工具名。
#[test]
fn the_tool_declares_its_behavior() {
    let meta = NotesTool.meta();
    assert_eq!(meta.name, "notes_add");
    assert!(meta.declares(behaviors::NOTE_CAPTURE));
    assert_eq!(meta.category, Some(CapabilityCategory::Other));
}
