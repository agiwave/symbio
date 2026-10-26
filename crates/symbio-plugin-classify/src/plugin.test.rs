//! `crates/symbio-plugin-classify/src/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 本文件覆盖的是「插件这一侧」的契约
//!
//! D-122 决策 2 之后，判决路由的出参**不是判决**，而是 [`DecideOutcome`]：
//! **判决**或**待跑的计划**。于是本文件钉住的是三条：
//!
//! 1. 规则表命中 ⇒ `Decided{Answered}`，**零模型往返**（上下文里连模型服务都没有）；
//! 2. 规则表未命中 ⇒ `NeedsModel{plan}`——**不是**判决：「判不判得出来」与
//!    「要不要打模型」在调用前是**两件事**，要不要派发由编排层决定；
//! 3. 没有用户发言 ⇒ `Decided{Escalate{unclassified}}`，绝不 `Answered`。
//!
//! 「没有模型服务 ⇒ `Escalate`」这条兜底**搬到了编排层**（`chat_loop/decide.rs`）：
//! 插件侧不再打模型 ⇒ 它没有「打不打得到模型」这个知识。
//!
//! 走真实模型的路径（快速档分类请求）由 e2e 覆盖，见 `e2e/cases/t21-classify.mjs`。

use super::*;
use symbio_core::{
    DecideOutcome, DecisionPlan, ParseVerdictRequest, PluginSimpleRequest, REASON_ACK,
    REASON_EMPTY, REASON_FROM_CONTEXT, REASON_GREETING, REASON_NEEDS_WORK, REASON_THANKS,
};

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
fn ctx<T: Clone + Send + Sync + 'static>(
    path: &str,
    payload: Option<T>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    if let Some(p) = payload {
        req.set_payload(p).expect("载荷写入必须成功");
    }
    Arc::new(req)
}

/// 不带载荷的请求上下文（缺载荷的路径用它）
fn bare_ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    ctx::<Vec<u8>>(path, None)
}

fn request(utterance: Option<&str>) -> DecideRequest {
    DecideRequest {
        session_id: "s1".to_string(),
        utterance: utterance.map(str::to_string),
        context: Vec::new(),
    }
}

fn parse_request(text: &str) -> ParseVerdictRequest {
    ParseVerdictRequest {
        session_id: "s1".to_string(),
        text: text.to_string(),
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
async fn decide(plugin: Arc<ClassifyPlugin>, utterance: Option<&str>) -> DecideOutcome {
    let p = plugin
        .route(ctx("decide", Some(request(utterance))))
        .await
        .expect("decide 必须成功");
    serde_json::from_value(data_of(p)).expect("出参必须能反序列化回契约类型")
}

/// 解析一次并回读判决
async fn parse(plugin: Arc<ClassifyPlugin>, text: &str) -> Verdict {
    let p = plugin
        .route(ctx("parse", Some(parse_request(text))))
        .await
        .expect("parse 必须成功");
    serde_json::from_value(data_of(p)).expect("解析结果必须能反序列化回判决")
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

/// 规则表命中的判据：**判决已经出来了**，而上下文里连模型服务都没有。
///
/// 「零 LLM 往返」的可检验形式正是这一条：如果命中之后还有一次模型调用，
/// 这个测试会拿到 `NeedsModel`（因为没有 provider 可解析），而不是 `Decided`。
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
            DecideOutcome::Decided {
                verdict: answered(reason)
            },
            "「{utterance}」应被规则表直接判决"
        );
    }
}

/// 规则表未命中 ⇒ 交出的是**计划**，不是判决。
///
/// 判据刻意是「没有判决」：插件侧不再打模型，于是「判不判得出来」与「要不要打
/// 模型」在调用前分成两件事——后者归编排层。少给一个计划，编排层就没有那次往返
/// 可派发；多给一个判决，编排层会以为「判完了」，那一次模型往返就被吞掉。
#[tokio::test]
async fn rule_miss_hands_over_a_plan_instead_of_a_verdict() {
    let plugin = plugin(ClassifyConfig::default());
    let outcome = decide(plugin, Some("我们刚才聊了什么")).await;
    let DecideOutcome::NeedsModel { plan } = outcome else {
        panic!("规则表未命中时应当交出计划，实得 {outcome:?}");
    };
    assert_plan_is_usable(&plan, "我们刚才聊了什么");
}

/// 计划是**数据**：编排层拿到它不必懂分类，只需照着打。
///
/// 三件东西缺一，编排层就得反过来猜——而「猜」正是 D-122 否掉的候选 ②。
fn assert_plan_is_usable(plan: &DecisionPlan, utterance: &str) {
    assert!(
        !plan.system_prompt.trim().is_empty(),
        "计划必须带系统提示词，否则编排层无从知道这次往返该怎么打"
    );
    assert!(
        plan.messages.iter().any(|m| m.text == utterance),
        "计划的消息里必须带着本轮这句话：{plan:?}"
    );
    assert!(
        plan.messages.last().map(|m| m.role.as_str()) == Some("user"),
        "本轮这句话必须是最后一条（分类只看最后一句）"
    );
}

/// 没有用户发言（后台触发的判定）：无事可判 ⇒ 直接给判决，且**绝不** `Answered`。
///
/// 它是**零往返**的一种（连计划都不给）——判不出来时 `Answered` 会让「用户什么都
/// 收不到」这件事静默发生。
#[tokio::test]
async fn absent_utterance_escalates() {
    let plugin = plugin(ClassifyConfig::default());
    assert_eq!(
        decide(plugin, None).await,
        DecideOutcome::Decided {
            verdict: escalated(REASON_UNCLASSIFIED)
        }
    );
}

/// 平凡值 `rule_shortcut = false`：**同一句话**不再被规则表短路。
///
/// 判据刻意选「同一句话、两种配置」——这样断言抓的是配置的效果本身，
/// 而不是「某句话恰好被判成什么」。没有模型服务时它交出计划（编排层去派发），
/// 与开关打开时的 `Decided{Answered}` 形成对照。
#[tokio::test]
async fn rule_shortcut_off_disables_the_table() {
    let on = plugin(ClassifyConfig::default());
    assert_eq!(
        decide(on, Some("你好")).await,
        DecideOutcome::Decided {
            verdict: answered(REASON_GREETING)
        }
    );

    let off = plugin(ClassifyConfig {
        rule_shortcut: false,
        ..Default::default()
    });
    assert!(
        matches!(
            decide(off, Some("你好")).await,
            DecideOutcome::NeedsModel { .. }
        ),
        "关掉规则短路后，问候不该再被规则表判成判决"
    );
}

/// 出参是**带判别键的枚举**：编排层据此分派，不解析文本。
#[tokio::test]
async fn the_outcome_serializes_as_a_tagged_enum() {
    let json = serde_json::to_value(DecideOutcome::Decided {
        verdict: answered(REASON_GREETING),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "outcome": "decided",
            "verdict": { "verdict": "answered", "reason": REASON_GREETING }
        })
    );

    let json = serde_json::to_value(DecideOutcome::NeedsModel {
        plan: DecisionPlan {
            system_prompt: "p".to_string(),
            messages: Vec::new(),
            model_id: None,
        },
    })
    .unwrap();
    assert_eq!(json["outcome"], serde_json::json!("needs_model"));
    assert_eq!(json["plan"]["system_prompt"], serde_json::json!("p"));
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

/// 解析路由：把分类器的自由文本变成闭集判决。
///
/// 词表是**本插件**的内部知识，所以解析也留在本插件——编排层只认枚举，不认那四个词。
#[tokio::test]
async fn the_parse_route_turns_a_word_into_a_verdict() {
    let plugin = plugin(ClassifyConfig::default());
    assert_eq!(
        parse(plugin.clone(), "direct").await,
        answered(REASON_FROM_CONTEXT)
    );
    assert_eq!(
        parse(plugin.clone(), "work").await,
        escalated(REASON_NEEDS_WORK)
    );
    // 多说几句也能救回来：词表里最早出现的那个词算数。
    assert_eq!(
        parse(plugin.clone(), "我觉得应该是 work 吧").await,
        escalated(REASON_NEEDS_WORK)
    );
}

/// 解析不成 ⇒ 「判不出来」，方向是 `Escalate`（绝不 `Answered`）。
#[tokio::test]
async fn an_unparsable_reply_falls_back_to_escalate() {
    let plugin = plugin(ClassifyConfig::default());
    assert_eq!(
        parse(plugin, "这段话里没有那四个词").await,
        escalated(REASON_UNCLASSIFIED)
    );
}

/// 契约缺载荷必须报错，而不是静默给一个默认判决——
/// 静默默认会让「忘了传请求」表现成「判决说不用干活」。
#[tokio::test]
async fn decide_without_payload_fails() {
    let r = plugin(ClassifyConfig::default())
        .route(bare_ctx("decide"))
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
    let outcome: DecideOutcome = serde_json::from_value(data_of(p)).unwrap();
    assert_eq!(
        outcome,
        DecideOutcome::Decided {
            verdict: answered(REASON_GREETING)
        }
    );
}

#[tokio::test]
async fn unknown_subcommand_is_not_found() {
    let r = plugin(ClassifyConfig::default())
        .route(bare_ctx("bogus"))
        .await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
}

/// 本插件不注册 `Capability` —— 它在工具的**注册表**里恒为零行。
///
/// 判据刻意数注册表而不是数 `traverse` 的回包：模型能调用的工具来自
/// `Registry::list(family::TOOL)`（`tool_metas` 读的就是它），回包是 `[]`
/// 只说明「没往回包里写东西」，注册表为零才说明「没进模型的工具清单」。
/// 这条是 [D-096](../../symbio-plugin-model/README.md#d-096-快速档裁-ii-一次受限模型调用classifyonly-令牌闸门显式退役不得生成的强制力搬到返回类型与工具集判据上 "crates/symbio-plugin-model/src/README") 给「快速档不得生成」
/// 配的第三颗牙：原来靠「本插件恰好不注册能力」这个结构巧合，现在是可执行断言。
///
/// 对照项是同一份 ctx 上的 `CONFIGURABLE_VISITOR`：它必须恰好收到 1 行。有这一行，
/// 上面的 0 才不是「收集器根本没接上」的同义反复。
#[tokio::test]
async fn traverse_registers_no_tool_rows() {
    use symbio_core::{
        capability_configurables, family, PluginInvokeRequestExt, Registry, CAPABILITY_VISITOR,
        CONFIGURABLE_VISITOR,
    };
    use symbio_providers::DefaultToolVisitor;

    let tools: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    let configurables: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, String::new());
    req.set(CAPABILITY_VISITOR, tools.clone());
    req.set(CONFIGURABLE_VISITOR, configurables.clone());
    let req: Arc<dyn PluginInvokeRequest> = Arc::new(req);

    let p = plugin(ClassifyConfig::default())
        .traverse(String::new(), req)
        .await
        .expect("traverse 必须成功");
    // 回包里也不许出现工具行（两处一起守：回包是广播面，注册表是模型面）
    assert_eq!(data_of(p), serde_json::json!([]));

    assert_eq!(
        tools.list(family::TOOL).await.len(),
        0,
        "classify 不得向工具注册表写任何行——那正是模型工具清单的来源"
    );
    assert_eq!(
        capability_configurables(&*configurables).await.len(),
        1,
        "对照：同一份 ctx 上配置通道必须收到 1 行，否则上面的 0 不可信"
    );
}

/// 但它**声明了自己的配置文档**（另一条通道）：设置页据此列出并指路
/// `<根>/classify/PLUGIN.yml`——「可 A/B」要能操作，靠的就是这一条。
///
/// 条目名用**目录名**（列表内唯一），地址必须显式带着 `classify/PLUGIN.yml`
/// （它跨挂载点、推不出来，见 `capability_entry_of` 的说明）。
#[tokio::test]
async fn traverse_announces_its_own_config() {
    use symbio_core::{
        capability_configurables, PluginInvokeRequestExt, Registry, CONFIGURABLE_VISITOR,
    };
    use symbio_providers::DefaultToolVisitor;

    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, String::new());
    req.set(CONFIGURABLE_VISITOR, visitor.clone());
    let req: Arc<dyn PluginInvokeRequest> = Arc::new(req);

    plugin(ClassifyConfig::default())
        .traverse(String::new(), req)
        .await
        .expect("traverse 必须成功");

    let items = capability_configurables(&*visitor).await;
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
