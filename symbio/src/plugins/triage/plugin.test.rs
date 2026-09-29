//! `symbio/src/plugins/triage/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 本文件覆盖的是「不依赖模型」的那一半
//!
//! 测试上下文**不带 `CAPABILITY_VISITOR`**——这正是「没有可用的模型服务」的真实
//! 形态。于是这里能钉住两条最重要的行为，且**不需要任何 mock**：
//!
//! 1. 规则命中时，**没有模型也能给出判决**（= 零 LLM 往返的等价证明：连模型都没有，
//!    判决照样出来了，说明这条路上根本没碰模型）；
//! 2. 规则未命中时，兜底方向是 `Escalate`（= 今天的行为），**绝不是 `Answered`**。
//!
//! 走真实模型的路径（快速档分类请求）由 e2e 覆盖，见 `e2e/cases/t21-triage.mjs`。

use super::super::reasons::{REASON_ACK, REASON_EMPTY, REASON_GREETING, REASON_THANKS};
use super::*;
use crate::symbio_core::PluginSimpleRequest;

/// 带路由路径 + 契约载荷的请求上下文（与容器转发的形状一致；**不带**能力访问器）
fn ctx(path: &str, payload: Option<DecideRequest>) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    if let Some(p) = payload {
        req.set_payload(p).expect("载荷写入必须成功");
    }
    Arc::new(req)
}

fn request(utterance: Option<&str>) -> DecideRequest {
    DecideRequest {
        session_id: "s1".to_string(),
        utterance: utterance.map(str::to_string),
        context: Vec::new(),
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

/// 判一次并回读契约类型
async fn decide(plugin: Arc<TriagePlugin>, utterance: Option<&str>) -> Verdict {
    let p = plugin
        .route(ctx("decide", Some(request(utterance))))
        .await
        .expect("decide 必须成功");
    serde_json::from_value(data_of(p)).expect("判决必须能反序列化回契约类型")
}

fn answered(reason: &str) -> Verdict {
    Verdict::Answered {
        reason: reason.to_string(),
    }
}

fn escalated(reason: &str) -> Verdict {
    Verdict::Escalate {
        reason: reason.to_string(),
    }
}

/// **S2 的核心判据**：规则命中 ⇒ 判决出来了，而上下文里连模型服务都没有。
///
/// 「零 LLM 往返」的可检验形式正是这一条：如果规则短路之外还有一次模型调用，
/// 这个测试会拿到 `Escalate{unclassified}`（因为没有 provider 可解析），而不是
/// `Answered`。
#[tokio::test]
async fn rule_hits_without_any_model_service() {
    let plugin = Arc::new(TriagePlugin::new(TriageConfig::default()));
    for (utterance, reason) in [
        ("你好", REASON_GREETING),
        ("谢谢", REASON_THANKS),
        ("好的", REASON_ACK),
        ("   ", REASON_EMPTY),
    ] {
        assert_eq!(
            decide(plugin.clone(), Some(utterance)).await,
            answered(reason),
            "「{utterance}」应被规则表直接判决"
        );
    }
}

/// 规则未命中且**没有模型服务** ⇒ 落 `Escalate`（= 今天的行为），不是 `Answered`。
///
/// 这条是失败方向的守卫：`Answered` 会让「判不出来」表现成**用户什么都收不到**。
#[tokio::test]
async fn rule_miss_without_model_service_escalates() {
    let plugin = Arc::new(TriagePlugin::new(TriageConfig::default()));
    assert_eq!(
        decide(plugin, Some("我们刚才聊了什么")).await,
        escalated(REASON_UNCLASSIFIED)
    );
}

/// 没有用户发言（后台触发的判定）：无事可判 ⇒ 同一条兜底方向
#[tokio::test]
async fn absent_utterance_escalates() {
    let plugin = Arc::new(TriagePlugin::new(TriageConfig::default()));
    assert_eq!(decide(plugin, None).await, escalated(REASON_UNCLASSIFIED));
}

/// 平凡值 `rule_shortcut = false`：**同一句话**不再被规则表短路。
///
/// 判据刻意选「同一句话、两种配置」——这样断言抓的是配置的效果本身，
/// 而不是「某句话恰好被判成什么」。没有模型服务时它落 `Escalate{unclassified}`，
/// 与开关打开时的 `Answered{greeting}` 形成对照。
#[tokio::test]
async fn rule_shortcut_off_disables_the_table() {
    let on = Arc::new(TriagePlugin::new(TriageConfig::default()));
    assert_eq!(decide(on, Some("你好")).await, answered(REASON_GREETING));

    let off = Arc::new(TriagePlugin::new(TriageConfig {
        rule_shortcut: false,
    }));
    assert_eq!(
        decide(off, Some("你好")).await,
        escalated(REASON_UNCLASSIFIED),
        "关掉规则短路后，问候不该再被规则表判成 Answered"
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
    let r = Arc::new(TriagePlugin::new(TriageConfig::default()))
        .route(ctx("decide", None))
        .await;
    assert!(r.is_err(), "缺载荷应报错，实得 {:?}", r.is_ok());
}

/// 契约扩展不破坏老调用方：只带 S1 那两个字段的载荷必须仍可解析
#[tokio::test]
async fn legacy_payload_without_new_fields_still_parses() {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, "decide".to_string());
    req.set_payload(serde_json::json!({ "session_id": "s1", "utterance": "你好" }))
        .expect("载荷写入必须成功");
    let p = Arc::new(TriagePlugin::new(TriageConfig::default()))
        .route(Arc::new(req))
        .await
        .expect("decide 必须成功");
    let v: Verdict = serde_json::from_value(data_of(p)).unwrap();
    assert_eq!(v, answered(REASON_GREETING));
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = Arc::new(TriagePlugin::new(TriageConfig::default()))
        .route(ctx("bogus", None))
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具集里**结构上不可能**出现
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = Arc::new(TriagePlugin::new(TriageConfig::default()))
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
