//! `symbio/src/plugins/reply/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::symbio_core::schemas::dialog::Verdict;
use crate::symbio_core::PluginSimpleRequest;

/// 带路由路径 + 契约载荷的请求上下文（与容器转发的形状一致）
fn ctx(path: &str, payload: Option<ComposeRequest>) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    if let Some(p) = payload {
        req.set_payload(p).expect("载荷写入必须成功");
    }
    Arc::new(req)
}

fn request(verdict: Verdict) -> ComposeRequest {
    ComposeRequest {
        session_id: "s1".to_string(),
        verdict,
        max_chars: None,
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

/// 本批（S1）的判据：平凡实现恒空串 —— 即「没有对话面文本」，
/// 与未挂载本插件时的行为**逐字一致**。三个判决变体都走同一条平凡路径。
#[tokio::test]
async fn compose_is_trivially_empty_for_every_verdict() {
    for v in [
        Verdict::Answered {
            reason: "greeting".to_string(),
        },
        Verdict::Escalate {
            reason: "unwired".to_string(),
        },
        Verdict::Report,
    ] {
        let p = Arc::new(ReplyPlugin)
            .route(ctx("compose", Some(request(v.clone()))))
            .await
            .unwrap_or_else(|e| panic!("compose({v:?}) 必须成功：{e}"));
        let text: String = serde_json::from_value(data_of(p)).expect("出参是 String");
        assert_eq!(text, "", "S1 的平凡实现必须恒空串（{v:?}）");
    }
}

/// 契约缺载荷必须报错，而不是静默给一个空串——
/// 静默默认会让「忘了传请求」表现成「措辞说没什么好说的」。
#[tokio::test]
async fn compose_without_payload_fails() {
    let r = Arc::new(ReplyPlugin).route(ctx("compose", None)).await;
    assert!(r.is_err(), "缺载荷应报错，实得 {:?}", r.is_ok());
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = Arc::new(ReplyPlugin).route(ctx("bogus", None)).await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具集里**结构上不可能**出现
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = Arc::new(ReplyPlugin)
        .traverse(String::new(), ctx("", None))
        .await
        .expect("traverse 必须成功");
    assert_eq!(data_of(p), serde_json::json!([]));
}

/// E-001 的自证：`PluginMeta` 首参必须等于插件目录名（容器按目录名分发）
#[test]
fn meta_id_matches_plugin_dir() {
    assert_eq!(ReplyPlugin::metadata().id, PLUGIN_ID_REPLY);
    assert_eq!(PLUGIN_ID_REPLY, "reply");
}
