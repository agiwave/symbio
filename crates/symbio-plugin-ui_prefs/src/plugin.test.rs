//! `plugin.rs` 的单元测试 —— 配置挂载点读写、落盘与隐藏属性。

use super::*;
use symbio_plugin_api::{
    vdfs_context, PluginSimpleRequest, VdfsContent, VdfsProvider, VdfsRequest, PLUGIN_FILE,
};
use tempfile::TempDir;

fn plugin(tmp: &TempDir, config: UiPrefsConfig) -> Arc<UiPrefsPlugin> {
    Arc::new(UiPrefsPlugin::new(
        PluginDir::at(tmp.path().join(PLUGIN_ID_UI_PREFS), PLUGIN_ID_UI_PREFS),
        config,
    ))
}

fn vctx() -> symbio_plugin_api::VdfsContext {
    vdfs_context(&(Arc::new(PluginSimpleRequest::new(None, None)) as Arc<dyn PluginInvokeRequest>))
}

/// 挂载根下恒为一份配置文档（普通文件形态，ext = form）
#[tokio::test]
async fn mount_serves_the_config_document() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, UiPrefsConfig::default());

    let nodes = p
        .dispatch(&vctx(), "", VdfsRequest::List {})
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node.name, PLUGIN_FILE);
    assert_eq!(
        nodes[0].node.ext.as_deref(),
        Some(symbio_plugin_api::VDFS_EXT_FORM)
    );
}

/// 写入 → 配置槽更新 → 落盘 → 读回一致
#[tokio::test]
async fn write_updates_slot_and_persists() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, UiPrefsConfig::default());
    let ctx = vctx();

    let mut value = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    value["theme"] = serde_json::json!("dark");
    value["font_size"] = serde_json::json!("large");
    value["volume"] = serde_json::json!(0.8);
    let content = VdfsContent::text(value.to_string()).with_mime("application/json");
    p.dispatch(&ctx, PLUGIN_FILE, VdfsRequest::Write { content })
        .await
        .unwrap();

    assert_eq!(p.config.read().await.theme, "dark");
    assert_eq!(p.config.read().await.font_size, "large");
    assert_eq!(p.config.read().await.volume, 0.8);
    assert!(
        tmp.path()
            .join(PLUGIN_ID_UI_PREFS)
            .join(PLUGIN_FILE)
            .exists(),
        "配置必须落盘，重启后可重建权威值"
    );

    let text = p
        .dispatch(&ctx, PLUGIN_FILE, VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap()
        .as_text()
        .unwrap_or_default()
        .to_string();
    assert!(text.contains("\"dark\""));
    assert!(text.contains("\"large\""));
}

/// 无效写入被定义拒绝：配置槽与磁盘都不动
#[tokio::test]
async fn invalid_write_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, UiPrefsConfig::default());
    let ctx = vctx();

    let mut value = serde_json::to_value(UiPrefsConfig::default()).unwrap();
    value["theme"] = serde_json::json!("neon");
    let content = VdfsContent::text(value.to_string()).with_mime("application/json");
    assert!(p
        .dispatch(&ctx, PLUGIN_FILE, VdfsRequest::Write { content })
        .await
        .is_err());
    assert_eq!(p.config.read().await.theme, "light");
}

/// 配置型挂载点：不进侧边栏（入口在插件管理的外观分区）
#[test]
fn metadata_is_hidden() {
    let m = UiPrefsPlugin::metadata();
    assert_eq!(m.id, PLUGIN_ID_UI_PREFS);
    assert!(m.hidden, "重复入口会让人不知道该点哪一个");
}

/// 无自有协议路由
#[tokio::test]
async fn plugin_has_no_own_routes() {
    let p = plugin(&TempDir::new().unwrap(), UiPrefsConfig::default());
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, "ui_prefs/whatever".to_string());
    let err = p.route(ctx).await.unwrap_err();
    assert!(format!("{err:?}").contains("VDFS"));
}
