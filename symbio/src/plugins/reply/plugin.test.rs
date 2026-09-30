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
use crate::symbio_core::schemas::dialog::{RunSnapshot, Verdict};
use crate::symbio_core::PluginSimpleRequest;

use super::super::reasons::{
    REASON_ACK, REASON_EMPTY, REASON_GREETING, REASON_NEEDS_WORK, REASON_THANKS,
    REASON_UNCLASSIFIED,
};
use super::super::templates::{FALLBACK_ANSWERED, FALLBACK_ESCALATE};

/// 造一个插件实例。
///
/// 目录只用于**声明配置文档**（`traverse` 那条通道），本文件的用例不读文件系统，
/// 故给一个临时目录即可——真读盘的那条路径由 `config.rs` 的单测与 e2e 覆盖。
fn plugin(config: ReplyConfig) -> Arc<ReplyPlugin> {
    Arc::new(ReplyPlugin::new(
        config,
        PluginDir::at(std::env::temp_dir(), PLUGIN_ID_REPLY),
    ))
}

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
        snapshot: RunSnapshot::default(),
    }
}

/// 带运行现状的 `Report` 请求（`snapshot` 是汇报唯一读的字段）。
fn report(snapshot: RunSnapshot) -> ComposeRequest {
    ComposeRequest {
        session_id: "s1".to_string(),
        verdict: Verdict::Report,
        context: Vec::new(),
        snapshot,
    }
}

/// 走一次路由，取出参文本
async fn compose_with(req: ComposeRequest) -> String {
    let p = plugin(ReplyConfig::default())
        .route(ctx("compose", Some(req)))
        .await
        .unwrap_or_else(|e| panic!("compose 必须成功：{e}"));
    serde_json::from_value(data_of(p)).expect("出参是 String")
}

/// 走一次路由（只给判决，运行现状取平凡值）
async fn compose(verdict: Verdict) -> String {
    compose_with(request(verdict)).await
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

/// `Report` 的产线是**填表**：零 LLM 往返（本上下文没有模型服务，它照样拿到文本）。
///
/// 这是"汇报不花一次往返"的等价证明——若它走生成，这里拿到的会是兜底而非那句
/// 带数字的话。汇报的全部意义是**减少**用户等待，而一次加在等待期间的往返
/// 会把这件事反过来做。
#[tokio::test]
async fn report_is_filled_from_the_snapshot_without_any_model_service() {
    let text = compose_with(report(RunSnapshot {
        tool_rounds: 3,
        quiet_ms: 125_000,
    }))
    .await;

    assert!(
        text.contains('3') && text.contains("2 分钟"),
        "汇报必须把运行现状（轮次 / 静默时长）说进句子里，实得：{text}"
    );
}

/// `Report` 恒有话说——包括运行现状是平凡值的情形。
///
/// 与 `answered_never_yields_empty_text` 同一条性质：空串会让这一轮**彻底沉默**，
/// 而沉默是这里最坏的失败形态。`tool_rounds = 0` 在编排层不可达（汇报判定要求至少
/// 走完一轮），但契约的第二个调用方是**网关**（外部客户端可直接调 `reply/compose`），
/// 那句话在这里必须说得通，而不是渲染出"已完成 0 轮工具调用"。
#[tokio::test]
async fn report_never_yields_empty_text() {
    for snapshot in [
        RunSnapshot::default(),
        RunSnapshot {
            tool_rounds: 1,
            quiet_ms: 0,
        },
        RunSnapshot {
            tool_rounds: 0,
            quiet_ms: -1,
        },
    ] {
        let text = compose_with(report(snapshot.clone())).await;
        assert!(
            !text.trim().is_empty(),
            "`Report` 必须有话可说（snapshot={snapshot:?}）"
        );
    }
}

/// 契约缺载荷必须报错，而不是静默给一个空串——
/// 静默默认会让「忘了传请求」表现成「措辞说没什么好说的」。
#[tokio::test]
async fn compose_without_payload_fails() {
    let r = plugin(ReplyConfig::default())
        .route(ctx("compose", None))
        .await;
    assert!(r.is_err(), "缺载荷应报错，实得 {:?}", r.is_ok());
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = plugin(ReplyConfig::default())
        .route(ctx("bogus", None))
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具集里**结构上不可能**出现
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = plugin(ReplyConfig::default())
        .traverse(String::new(), ctx("", None))
        .await
        .expect("traverse 必须成功");
    assert_eq!(data_of(p), serde_json::json!([]));
}

/// 但它**声明了自己的配置文档**（另一条通道）：设置页据此列出并指路
/// `<根>/reply/PLUGIN.yml`——「可 A/B」要能操作，靠的就是这一条。
#[tokio::test]
async fn traverse_announces_its_own_config() {
    use crate::providers::DefaultConfigurableVisitor;
    use crate::symbio_core::{ConfigurableVisitor, CONFIGURABLE_VISITOR};

    let visitor: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, String::new());
    req.set(CONFIGURABLE_VISITOR, visitor.clone());
    let req: Arc<dyn PluginInvokeRequest> = Arc::new(req);

    plugin(ReplyConfig::default())
        .traverse(String::new(), req)
        .await
        .expect("traverse 必须成功");

    let items = visitor.list_configurables().await;
    assert_eq!(items.len(), 1, "应恰好声明一条配置文档");
    assert_eq!(items[0].node.name, PLUGIN_ID_REPLY);
    assert_eq!(items[0].path, "reply/PLUGIN.yml");
}

/// E-001 的自证：`PluginMeta` 首参必须等于插件目录名（容器按目录名分发）
#[test]
fn meta_id_matches_plugin_dir() {
    assert_eq!(ReplyPlugin::metadata().id, PLUGIN_ID_REPLY);
    assert_eq!(PLUGIN_ID_REPLY, "reply");
}
