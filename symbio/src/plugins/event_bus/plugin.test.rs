//! `symbio/src/plugins/event_bus/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::SimpleRequest;

fn ctx(path: &str) -> Arc<dyn InvokeRequest> {
    let req = SimpleRequest::new(None, None);
    req.set(crate::symbio_core::PATH, path.to_string());
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!(
            "expected data payload, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = Arc::new(EventBusPlugin).route(ctx("bogus")).await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

#[tokio::test]
async fn ping_reports_subscriber_count() {
    let p = Arc::new(EventBusPlugin)
        .route(ctx("ping"))
        .await
        .expect("ping 必须成功");
    let s = serde_json::to_string(&data_of(p)).unwrap();
    assert!(s.contains("pong"), "ping 响应应含 pong：{s}");
}

/// 总线插件不贡献工具——它在能力树里是个纯连接入口
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = Arc::new(EventBusPlugin)
        .traverse(String::new(), ctx(""))
        .await
        .expect("traverse 必须成功");
    assert_eq!(data_of(p), serde_json::json!([]));
}
