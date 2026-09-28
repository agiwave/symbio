//! `setting/plugin.rs` 的单元测试 —— 收集期交出的两样东西与**缺省不注入**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{CapabilityVisitor, PluginInvokeRequestExt, VDFS_PARENT_ADDR};
use tempfile::TempDir;

/// 构造带能力收集器的上下文
fn ctx(parent_addr: &str) -> (Arc<dyn PluginInvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    ctx.set(VDFS_PARENT_ADDR, parent_addr.to_string());
    let visitor = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn CapabilityVisitor>,
    );
    (ctx, visitor)
}

fn plugin(tmp: &TempDir, config: SettingConfig) -> Arc<SettingPlugin> {
    Arc::new(SettingPlugin::new(
        crate::symbio_core::PluginDir::at(tmp.path().join(PLUGIN_ID_SETTING), PLUGIN_ID_SETTING),
        config,
    ))
}

/// 缺省（什么都没填）⇒ 不注入，但挂载点照旧交出
#[tokio::test]
async fn default_settings_inject_nothing_but_keep_the_mount() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, SettingConfig::default());
    let (ctx, visitor) = ctx("@vfs/setting");

    p.traverse(String::new(), ctx).await.unwrap();

    assert!(
        visitor.list_system_prompts().await.is_empty(),
        "缺省状态不该改变任何一轮的提示词"
    );
    assert!(visitor.get_vdfs_provider(PLUGIN_ID_SETTING).await.is_some());
}

/// 填了档案 ⇒ 注入一段【本智能体】
#[tokio::test]
async fn filled_profile_is_injected() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(
        &tmp,
        SettingConfig {
            display_name: "评审官".to_string(),
            reply_language: "中文".to_string(),
            ..SettingConfig::default()
        },
    );
    let (ctx, visitor) = ctx("@vfs/setting");

    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].0, segment::SEGMENT_NAME);
    assert!(segs[0].1.contains("名称：评审官"), "{}", segs[0].1);
    assert!(segs[0].1.contains("回复语言：中文"), "{}", segs[0].1);
}

/// 挂载点是**纯配置挂载点**：根下列出配置文档，按真实文件名可读写
#[tokio::test]
async fn mount_serves_the_config_document() {
    use crate::symbio_core::{vdfs_context, VdfsProvider, VdfsRequest};

    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, SettingConfig::default());
    let vctx = vdfs_context(
        &(Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None))
            as Arc<dyn PluginInvokeRequest>),
    );

    let nodes = p
        .dispatch(
            &vctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(nodes.len(), 1, "挂载根下恒为一份配置文档");
    assert_eq!(nodes[0].node.name, crate::symbio_core::PLUGIN_FILE);
    assert_eq!(
        nodes[0].node.ext.as_deref(),
        Some(crate::symbio_core::VDFS_EXT_FORM),
        "配置文档渲染为表单"
    );
}

// ==================== 无自有路由 ====================

#[tokio::test]
async fn plugin_has_no_own_routes() {
    let tmp = TempDir::new().unwrap();
    let p = plugin(&tmp, SettingConfig::default());
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    ctx.set(PATH, "setting/whatever".to_string());
    let err = p.route(ctx).await.unwrap_err();
    assert!(
        format!("{err:?}").contains("VDFS"),
        "错误信息要指引到 VDFS：{err:?}"
    );
}
