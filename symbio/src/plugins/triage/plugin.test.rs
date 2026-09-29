//! `symbio/src/plugins/triage/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::symbio_core::PluginSimpleRequest;

/// 带路由路径 + 契约载荷的请求上下文（与容器转发的形状一致）
fn ctx(path: &str, payload: Option<DecideRequest>) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    if let Some(p) = payload {
        req.set_payload(p).expect("载荷写入必须成功");
    }
    Arc::new(req)
}

fn request() -> DecideRequest {
    DecideRequest {
        session_id: "s1".to_string(),
        utterance: Some("你好".to_string()),
    }
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

/// 本批（S1）的判据：平凡实现恒 `Escalate` —— 即「全部输入进工具循环」，
/// 与未挂载本插件时的行为**逐字一致**。
#[tokio::test]
async fn decide_is_trivially_escalate() {
    let p = Arc::new(TriagePlugin)
        .route(ctx("decide", Some(request())))
        .await
        .expect("decide 必须成功");

    let v: Verdict = serde_json::from_value(data_of(p)).expect("判决必须能反序列化回契约类型");
    assert_eq!(
        v,
        Verdict::Escalate {
            reason: REASON_UNWIRED.to_string()
        },
        "S1 的平凡实现必须恒 Escalate"
    );
}

/// 判决是**枚举**：序列化形态带 `verdict` 判别键（编排层据此分派，不解析文本）
#[tokio::test]
async fn verdict_serializes_as_tagged_enum() {
    let json = serde_json::to_value(Verdict::Report).unwrap();
    assert_eq!(json, serde_json::json!({ "verdict": "report" }));

    let json = serde_json::to_value(Verdict::Answered {
        reason: "greeting".to_string(),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "verdict": "answered", "reason": "greeting" })
    );
}

/// 契约缺载荷必须报错，而不是静默给一个默认判决——
/// 静默默认会让「忘了传请求」表现成「判决说不用干活」。
#[tokio::test]
async fn decide_without_payload_fails() {
    let r = Arc::new(TriagePlugin).route(ctx("decide", None)).await;
    assert!(r.is_err(), "缺载荷应报错，实得 {:?}", r.is_ok());
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = Arc::new(TriagePlugin).route(ctx("bogus", None)).await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具集里**结构上不可能**出现
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = Arc::new(TriagePlugin)
        .traverse(String::new(), ctx("", None))
        .await
        .expect("traverse 必须成功");
    assert_eq!(data_of(p), serde_json::json!([]));
}

/// E-001 的自证：`PluginMeta` 首参必须等于插件目录名（容器按目录名分发）
#[test]
fn meta_id_matches_plugin_dir() {
    assert_eq!(TriagePlugin::metadata().id, PLUGIN_ID_TRIAGE);
    assert_eq!(PLUGIN_ID_TRIAGE, "triage");
}
