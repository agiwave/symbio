//! `symbio/src/plugins/reply/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 这些用例为什么全部不依赖模型
//!
//! 测试上下文里**没有** `CAPABILITY_VISITOR`（它由 `session` 经容器广播挂上）。
//! 这恰好让"模板产线零 LLM 往返"成为可执行的判据：模板路径**根本不碰**模型服务，
//! 因此在这里能拿到正确文本；而生成路径取不到模型服务 ⇒ 走兜底。
//! 生成路径本身由 e2e `t22-reply.mjs` 在真实边界上验（mock LLM）。

use super::*;
use crate::symbio_core::schemas::dialog::Verdict;
use crate::symbio_core::PluginSimpleRequest;

use super::super::reasons::{
    REASON_ACK, REASON_EMPTY, REASON_GREETING, REASON_NEEDS_WORK, REASON_THANKS,
    REASON_UNCLASSIFIED,
};
use super::super::templates::{FALLBACK_ANSWERED, FALLBACK_ESCALATE};

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
        context: Vec::new(),
    }
}

/// 走一次路由，取出参文本
async fn compose(verdict: Verdict) -> String {
    let p = Arc::new(ReplyPlugin)
        .route(ctx("compose", Some(request(verdict))))
        .await
        .unwrap_or_else(|e| panic!("compose 必须成功：{e}"));
    let text: String = serde_json::from_value(data_of(p)).expect("出参是 String");
    text
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

fn answered(reason: &str) -> Verdict {
    Verdict::Answered {
        reason: reason.to_string(),
    }
}

fn escalate(reason: &str) -> Verdict {
    Verdict::Escalate {
        reason: reason.to_string(),
    }
}

/// **模板产线零 LLM 往返**：本测试上下文没有模型服务，而模板理由码照样拿到文本。
///
/// 这是"模板路径不碰模型"的等价证明——如果它需要模型，这里会落兜底而不是拿到
/// 那句具体的模板。
#[tokio::test]
async fn template_reasons_resolve_without_any_model_service() {
    assert_eq!(compose(answered(REASON_THANKS)).await, "不客气。");
    assert_eq!(
        compose(answered(REASON_GREETING)).await,
        "你好，我在。有什么事直接说就行。"
    );
    assert_eq!(compose(answered(REASON_ACK)).await, "收到。");
    assert_eq!(compose(escalate(REASON_NEEDS_WORK)).await, "好，我来处理。");
    assert_eq!(compose(escalate(REASON_UNCLASSIFIED)).await, "我先看一下。");
}

/// **`Answered` 恒有话说**（本插件在场时）。
///
/// 这条是本批最要紧的性质：判决说"能直接答"，用户就**必须**收到一句答话。
/// 生成路径失败（没有模型服务）时落到变体兜底，而不是空串——
/// 空串会让这一轮**彻底沉默**，而沉默是这里最坏的失败形态（没有任何错误信号）。
#[tokio::test]
async fn answered_never_yields_empty_text() {
    for reason in [
        REASON_THANKS,
        REASON_GREETING,
        REASON_ACK,
        REASON_EMPTY,
        // 走生成产线，但本上下文没有模型服务 ⇒ 落到兜底
        "from_context",
        // 未知码 ⇒ 落到兜底
        "未来才有的码",
    ] {
        let text = compose(answered(reason)).await;
        assert!(
            !text.trim().is_empty(),
            "`Answered {{ reason: {reason} }}` 必须有话可说"
        );
    }
}

/// 生成路径拿不到模型服务时，落**`Answered` 的**兜底（不是空串、也不是 `Escalate` 的口吻）。
#[tokio::test]
async fn from_context_without_a_model_falls_back_to_the_answered_phrasing() {
    assert_eq!(compose(answered("from_context")).await, FALLBACK_ANSWERED);
}

/// 未知理由码按**变体**兜底：`Escalate` 的口吻必须是"我来处理"。
#[tokio::test]
async fn unknown_reason_falls_back_by_variant() {
    assert_eq!(compose(answered("未来才有的码")).await, FALLBACK_ANSWERED);
    assert_eq!(compose(escalate("未来才有的码")).await, FALLBACK_ESCALATE);
}

/// `Report` 本批**没有产线** ⇒ 空串（= 没有对话面文本，平凡值）。
///
/// 它的措辞要从**运行现状**组织（在跑什么、跑了多久），而那个快照到 S4 才有生产者。
/// 本批**不编**一句话：编出来的那句必然与界面上的真实进展不符，比不说更糟。
#[tokio::test]
async fn report_yields_no_dialog_text_yet() {
    assert_eq!(compose(Verdict::Report).await, "");
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
