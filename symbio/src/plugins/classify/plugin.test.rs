//! `symbio/src/plugins/classify/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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
//! 走真实模型的路径（快速档分类请求）由 e2e 覆盖，见 `e2e/cases/t21-classify.mjs`。

use super::super::reasons::{REASON_ACK, REASON_EMPTY, REASON_GREETING, REASON_THANKS};
use super::*;
use crate::symbio_core::PluginSimpleRequest;

/// 造一个插件实例。
///
/// 目录只用于**声明配置文档**（`traverse` 那条通道），本文件的用例不读文件系统，
/// 故给一个临时目录即可——真读盘的那条路径由 `config.rs` 的单测与 e2e 覆盖。
fn plugin(config: ClassifyConfig) -> Arc<ClassifyPlugin> {
    Arc::new(ClassifyPlugin::new(
        config,
        PluginDir::at(std::env::temp_dir(), PLUGIN_ID_CLASSIFY),
    ))
}

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
async fn decide(plugin: Arc<ClassifyPlugin>, utterance: Option<&str>) -> Verdict {
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
    let plugin = plugin(ClassifyConfig::default());
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
    let plugin = plugin(ClassifyConfig::default());
    assert_eq!(
        decide(plugin, Some("我们刚才聊了什么")).await,
        escalated(REASON_UNCLASSIFIED)
    );
}

/// 没有用户发言（后台触发的判定）：无事可判 ⇒ 同一条兜底方向
#[tokio::test]
async fn absent_utterance_escalates() {
    let plugin = plugin(ClassifyConfig::default());
    assert_eq!(decide(plugin, None).await, escalated(REASON_UNCLASSIFIED));
}

/// 平凡值 `rule_shortcut = false`：**同一句话**不再被规则表短路。
///
/// 判据刻意选「同一句话、两种配置」——这样断言抓的是配置的效果本身，
/// 而不是「某句话恰好被判成什么」。没有模型服务时它落 `Escalate{unclassified}`，
/// 与开关打开时的 `Answered{greeting}` 形成对照。
#[tokio::test]
async fn rule_shortcut_off_disables_the_table() {
    let on = plugin(ClassifyConfig::default());
    assert_eq!(decide(on, Some("你好")).await, answered(REASON_GREETING));

    let off = plugin(ClassifyConfig {
        rule_shortcut: false,
        ..Default::default()
    });
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
    let r = plugin(ClassifyConfig::default())
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
    let p = plugin(ClassifyConfig::default())
        .route(Arc::new(req))
        .await
        .expect("decide 必须成功");
    let v: Verdict = serde_json::from_value(data_of(p)).unwrap();
    assert_eq!(v, answered(REASON_GREETING));
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = plugin(ClassifyConfig::default())
        .route(ctx("bogus", None))
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具集里**结构上不可能**出现
#[tokio::test]
async fn traverse_contributes_no_tools() {
    let p = plugin(ClassifyConfig::default())
        .traverse(String::new(), ctx("", None))
        .await
        .expect("traverse 必须成功");
    assert_eq!(data_of(p), serde_json::json!([]));
}

/// 但它**声明了自己的配置文档**（另一条通道）：设置页据此列出并指路
/// `<根>/classify/PLUGIN.yml`——「可 A/B」要能操作，靠的就是这一条。
///
/// 条目名用**目录名**（列表内唯一），地址必须显式带着 `classify/PLUGIN.yml`
/// （它跨挂载点、推不出来，见 `capability_entry_of` 的说明）。
#[tokio::test]
async fn traverse_announces_its_own_config() {
    use crate::providers::DefaultConfigurableVisitor;
    use crate::symbio_core::{ConfigurableVisitor, CONFIGURABLE_VISITOR};

    let visitor: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, String::new());
    req.set(CONFIGURABLE_VISITOR, visitor.clone());
    let req: Arc<dyn PluginInvokeRequest> = Arc::new(req);

    plugin(ClassifyConfig::default())
        .traverse(String::new(), req)
        .await
        .expect("traverse 必须成功");

    let items = visitor.list_configurables().await;
    assert_eq!(items.len(), 1, "应恰好声明一条配置文档");
    assert_eq!(items[0].node.name, PLUGIN_ID_CLASSIFY);
    assert_eq!(items[0].path, "classify/PLUGIN.yml");
    // 表单定义随声明一起交出去（设置页不另查一份）
    let keys: Vec<&str> = items[0]
        .node
        .schema
        .as_ref()
        .and_then(|s| s.get("sections"))
        .and_then(|s| s.get(0))
        .and_then(|s| s.get("fields"))
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f.get("key").and_then(|k| k.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(keys, vec!["rule_shortcut", "model", "system_prompt"]);
}

/// E-001 的自证：`PluginMeta` 首参必须等于插件目录名（容器按目录名分发）
#[test]
fn meta_id_matches_plugin_dir() {
    assert_eq!(ClassifyPlugin::metadata().id, PLUGIN_ID_CLASSIFY);
    assert_eq!(PLUGIN_ID_CLASSIFY, "classify");
}
