//! projection 插件自测 —— 两条只读路由与平凡值。

use super::*;
use crate::symbio_core::{
    PluginInvokeRequest, PluginInvokeRequestExt, PluginPayload, PluginSimpleRequest, PATH,
    PLUGIN_PAYLOAD_KEY,
};
use serde_json::json;
use std::any::Any;
use std::sync::Arc;

fn plugin() -> ProjectionPlugin {
    let dir = crate::symbio_core::PluginDir::at(
        std::env::temp_dir().join("symbio-test-projection"),
        "projection",
    );
    ProjectionPlugin::new(dir, ProjectionConfig::default())
}

/// 造一个 invoke 上下文：带 PATH，可选带载荷（`PLUGIN_PAYLOAD_KEY` 原始桶）。
fn ctx(path: &str, payload: Option<serde_json::Value>) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    if let Some(p) = payload {
        req.set_raw(
            PLUGIN_PAYLOAD_KEY,
            Arc::new(p) as Arc<dyn Any + Send + Sync>,
        );
    }
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!("expected data, got {:?}", std::mem::discriminant(&other)),
    }
}

/// `list` 路由：返回投影名数组，且**至少**含会话三投影（它们已登记）。
#[tokio::test]
async fn list_route_returns_registered_names() {
    let p = Arc::new(plugin());
    let resp = p
        .clone()
        .route(ctx("list", None))
        .await
        .expect("list 必须成功");
    let v = data_of(resp);
    let arr = v.as_array().expect("list 返回应为数组");
    let names: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
    for expect in ["session.snapshot", "session.display", "session.checkpoint"] {
        assert!(names.contains(&expect), "应含已登记投影 {expect}");
    }
}

/// `run` 路由：跑一个已登记投影，回 `{ name, view }`，`view.value` 形状正确。
#[tokio::test]
async fn run_route_executes_projection() {
    let p = Arc::new(plugin());
    let body = json!({
        "name": "session.snapshot",
        "facts": [
            {"seq":1,"kind":"turn_user_message","principal":"s1",
             "caused_by":null,"at_ms":10,
             "payload":{"message_id":"u1","role":"User","type":"Text","timestamp":10}}
        ],
        "at_ms": 99
    });
    let resp = p
        .clone()
        .route(ctx("run", Some(body)))
        .await
        .expect("run 必须成功");
    let v = data_of(resp);
    assert_eq!(v["name"], "session.snapshot");
    assert_eq!(v["view"]["value"]["total"], 1, "应折叠出 1 条消息");
    assert_eq!(v["view"]["trivial"], false);
}

/// `run` 未知投影名 ⇒ `NotFound`（可区分"未接入"）。
#[tokio::test]
async fn run_unknown_projection_is_not_found() {
    let p = Arc::new(plugin());
    let body = json!({ "name": "__nope__" });
    let r = p.route(ctx("run", Some(body))).await;
    assert!(matches!(
        r,
        Err(crate::symbio_core::PluginError::NotFound(_))
    ));
}

/// `run` 无载荷 ⇒ 不报错，按空事实跑出确定结果（最小调用可用）。
#[tokio::test]
async fn run_without_payload_is_empty_not_error() {
    let p = Arc::new(plugin());
    let body = json!({ "name": "session.snapshot" });
    let resp = p
        .clone()
        .route(ctx("run", Some(body)))
        .await
        .expect("缺 facts 也应成功");
    let v = data_of(resp);
    assert_eq!(v["view"]["value"]["total"], 0);
}

/// **A4**：同一载荷双跑，逐字节相同。
#[tokio::test]
async fn run_is_deterministic() {
    let p = Arc::new(plugin());
    let body = json!({
        "name": "session.checkpoint",
        "facts": [
            {"seq":1,"kind":"turn_user_message","principal":"s1","caused_by":null,
             "at_ms":1,"payload":{"message_id":"u1","role":"User","type":"Text"}},
            {"seq":2,"kind":"turn_assistant_final","principal":"s1","caused_by":1,
             "at_ms":2,"payload":{"message_id":"a1","role":"Assistant","type":"Text"}}
        ],
        "at_ms": 5
    });
    let a = data_of(
        p.clone()
            .route(ctx("run", Some(body.clone())))
            .await
            .unwrap(),
    );
    let b = data_of(p.clone().route(ctx("run", Some(body))).await.unwrap());
    assert_eq!(a, b, "同一载荷双跑必须逐字节相同");
}

/// 未知子命令 ⇒ `NotFound`。
#[tokio::test]
async fn unknown_route_is_not_found() {
    let p = Arc::new(plugin());
    let r = p.route(ctx("bogus", None)).await;
    assert!(matches!(
        r,
        Err(crate::symbio_core::PluginError::NotFound(_))
    ));
}

/// **平凡值（J2）**：`enabled = false` ⇒ 两条路由都不可达，但**投影机制不受影响**
/// （`symbio_core::projection_list` 仍能列出投影——关掉的只是这个窗口）。
#[tokio::test]
async fn disabled_makes_routes_unreachable() {
    let dir = crate::symbio_core::PluginDir::at(
        std::env::temp_dir().join("symbio-test-projection-off"),
        "projection",
    );
    let p = Arc::new(ProjectionPlugin::new(
        dir,
        ProjectionConfig { enabled: false },
    ));

    for path in ["list", "run"] {
        let r = p.clone().route(ctx(path, None)).await;
        assert!(
            matches!(r, Err(crate::symbio_core::PluginError::NotFound(_))),
            "停用后 {path} 应不可达"
        );
    }
    // 反证：机制面不受影响
    assert!(
        crate::symbio_core::projection_has("session.snapshot"),
        "停用内省口不应影响投影登记"
    );
}
