//! `symbio/src/plugins/gateway/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::vdfs::{VdfsContext, VdfsError, VdfsProvider};
use crate::symbio_core::{InvokeRequestExt, InvokeResponse, Plugin, PluginPayload, SimpleRequest};

/// 以子插件直接收的**已剥离前缀**路径（如 `status`）调用 route，
/// 模拟 home composite 转发后的行为。
async fn call(
    plugin: Arc<GatewayPlugin>,
    path: &str,
    payload: Option<serde_json::Value>,
) -> InvokeResponse<PluginPayload> {
    let ctx = SimpleRequest::new(None, None);
    ctx.set(PATH, path.to_string());
    if let Some(p) = payload {
        ctx.set_payload(p).unwrap();
    }
    plugin.clone().route(Arc::new(ctx)).await
}

fn vctx() -> VdfsContext {
    VdfsContext::empty()
}

/// 测试用目录：这些用例都不触发落盘（坏值在写盘之前就被拦下），
/// 因此指向一个临时目录即可，不必真建
fn tdir() -> PluginDir {
    PluginDir::at(std::env::temp_dir().join("symbio-test-gateway"), "gateway")
}

/// 配置文档的形状：根下唯一一项、`ext = form`（前端据此选通用表单渲染器）、`rw`
#[tokio::test]
async fn config_document_is_the_only_child_of_the_root() {
    let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
    let items = plugin
        .dispatch(
            &vctx(),
            "",
            vdfs::VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, PLUGIN_FILE, "地址就是插件目录里的真实文件名");
    assert_eq!(items[0].title, "开放接口");
    assert_eq!(items[0].ext.as_deref(), Some(vdfs::VDFS_EXT_FORM));
    assert_eq!(items[0].access.flags(), "rw");
    assert!(items[0].schema.is_some(), "定义随节点下发");
}

/// 读：配置文档回当前配置（pretty JSON）
#[tokio::test]
async fn config_document_reads_current_config() {
    let cfg = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_port: 9231,
        ..GatewayConfig::default()
    };
    let plugin = Arc::new(GatewayPlugin::new(None, cfg, tdir()));
    let content = plugin
        .dispatch(&vctx(), PLUGIN_FILE, vdfs::VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    let got: GatewayConfig = serde_json::from_str(content.text.as_deref().unwrap()).unwrap();
    assert!(got.inbound_enabled);
    assert_eq!(got.inbound_protocol, "http");
    assert_eq!(got.inbound_port, 9231);
}

/// 写：校验先于一切——坏值在落内存之前就被定义拦下（字段级错误）
#[tokio::test]
async fn config_document_write_validates_before_applying() {
    let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
    let bad = vdfs::VdfsContent::text("", r#"{"inbound_port": 70000}"#);
    match plugin
        .dispatch(
            &vctx(),
            PLUGIN_FILE,
            vdfs::VdfsRequest::Write { content: bad },
        )
        .await
    {
        Err(VdfsError::Invalid(v)) => assert_eq!(v.fields[0].field, "inbound_port"),
        other => panic!("应为字段级校验错误，实得 {other:?}"),
    }
    // 未被改动
    assert_eq!(plugin.config.read().await.inbound_port, 9231);
}

#[tokio::test]
async fn status_reports_server_not_running_when_inbound_disabled() {
    let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
    let resp = call(plugin, "status", None).await.unwrap();
    let v = resp.serialize().unwrap();
    assert_eq!(v["inbound_running"], false);
    assert_eq!(v["inbound_enabled"], false);
}

#[tokio::test]
async fn unknown_path_is_not_found() {
    let plugin = Arc::new(GatewayPlugin::new(None, GatewayConfig::default(), tdir()));
    assert!(call(plugin.clone(), "bogus", None).await.is_err());
    // 配置文档之外无其它节点
    assert!(plugin
        .dispatch(&vctx(), "bogus", vdfs::VdfsRequest::Stat)
        .await
        .is_err());
}
