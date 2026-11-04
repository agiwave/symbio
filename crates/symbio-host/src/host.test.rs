//! 宿主的机制状态：无债即 `idle`，有未还债即撤销。

use super::*;
// 只做链接：环经 creator 注册被这个根构造，测试二进制必须把它的注册项带进来。
use symbio_deploy::PLUGIN_ID_LOOP;
use symbio_plugin_api::port::{
    port_rows, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::FactDraft;
use symbio_plugin_loop as _;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSpec, DebtSource, DriveRequest};
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};
use symbio_kernel::event::{Entity, Event, Verb};

/// 没有业务行、只把系统环挂进来的根：用来把驱动走到「无债」。
struct EmptyRoot;

#[async_trait::async_trait]
impl Plugin for EmptyRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("test-root", "test-root")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("route".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 与真实装配一致：系统插件从工厂表构造，再委托 traverse（根自己没有行）。
        let ring = symbio_plugin_api::creator_create_object::<dyn Plugin>(
            PLUGIN_ID_LOOP,
            Arc::clone(&ctx),
        )
        .ok_or_else(|| PluginError::NotFound("环插件未注册".into()))?;
        ring.traverse(path, ctx).await
    }
}

#[tokio::test]
async fn a_scope_with_no_debt_is_idle() {
    let host = Host::new(Arc::new(EmptyRoot));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host.drive(&base, "s-1", None).await.expect("驱动装配成立");

    assert_eq!(report.scope, scope_of("s-1"));
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    assert_eq!(report.dispatched, 0);
    assert!(host.idle_at(report.scope).is_some(), "无未还债 ⇒ 落 idle");
}

#[test]
fn scope_identity_never_degrades_to_zero() {
    assert_ne!(scope_of(""), 0, "空会话 id 也有确定身份");
    assert_ne!(scope_of("a"), scope_of("b"));
}

/// 最小 smoke 的模型端口：执行体只读事实、只落 `response`。
struct SmokeModel;

impl PortProvider for SmokeModel {
    fn ports(&self) -> &'static [&'static str] {
        &["model"]
    }

    fn provider_id(&self) -> &str {
        "smoke"
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(SmokeExecutor))
    }
}

struct SmokeExecutor;

#[async_trait::async_trait]
impl PortExecutor for SmokeExecutor {
    async fn execute(&self, exec: PortExecContext, _input: PortInput) -> PortOutcome {
        // 正文全从事实读面取（环只转交 reader）。
        let facts = exec
            .reader
            .as_ref()
            .map(|reader| reader.facts(exec.subject))
            .unwrap_or_default();
        let user = facts
            .iter()
            .find(|fact| fact.kind == EVENT_USER_MESSAGE)
            .and_then(|fact| fact.payload.get("text"))
            .and_then(|text| text.as_str())
            .unwrap_or_default();
        let draft = FactDraft {
            key: "smoke:response".into(),
            kind: symbio_plugin_api::EVENT_RESPONSE.into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "text": format!("answer: {user}") }),
            produced_by: None,
        };
        exec.emit.emit(vec![draft]).await;
        PortOutcome::ok()
    }
}

/// model 的两行债（与 `symbio-plugin-model` 的债表同形；smoke 只验机制链）。
struct SmokeDebts;

impl DebtSource for SmokeDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let payload = serde_json::json!({
            "provider": request.provider,
            "root_id": user_id,
            "principal": "user",
            "scope": request.scope,
        });
        let mut rows = vec![DebtSpec::new(
            "model-reply",
            "model",
            format!("model:{user_id}"),
            payload.clone(),
        )
        .when(EVENT_USER_MESSAGE)
        .unless_id(EVENT_CHAT_ASSISTANT_FINAL, format!("chat.assistant.final:{user_id}"))];
        if let Some(tool_id) = facts.latest_fact_id(symbio_plugin_api::events::EVENT_ARTIFACT_ADDED) {
            rows.push(
                DebtSpec::new("model-after-tools", "model", format!("model:after:{tool_id}"), payload)
                    .when_no_open("tool"),
            );
        }
        rows
    }
}

/// 环 + 模型端口 + 债表的根（装配走 creator 与能力收集，不另造注册面）。
struct SmokeRoot;

#[async_trait::async_trait]
impl Plugin for SmokeRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("smoke-root", "smoke-root")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("route".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let ring = symbio_plugin_api::creator_create_object::<dyn Plugin>(
            PLUGIN_ID_LOOP,
            Arc::clone(&ctx),
        )
        .ok_or_else(|| PluginError::NotFound("环插件未注册".into()))?;
        ring.traverse(path.clone(), Arc::clone(&ctx)).await?;

        let visitor = ctx
            .get(symbio_plugin_api::CAPABILITY_VISITOR)
            .ok_or_else(|| PluginError::NotFound("能力收集器缺席".into()))?;
        let provider: Arc<dyn PortProvider> = Arc::new(SmokeModel);
        for row in port_rows(&provider) {
            visitor.put(row).await;
        }
        visitor
            .put(debt_row("smoke-model", Arc::new(SmokeDebts)))
            .await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

#[tokio::test]
async fn user_message_becomes_a_model_debt_that_lands_its_response_in_the_store() {
    let session = "smoke-1";
    let scope = scope_of(session);

    // 世界事实由写方（会话侧扮演）落进事实源：宿主没有写口。
    let store = Arc::new(symbio_kernel::store::MemoryStore::<Event>::new());
    store
        .append(
            Event::pending(
                "u-1",
                EVENT_USER_MESSAGE,
                Entity::Scope,
                Verb::Opened,
                scope,
                "user",
            )
            .with_payload(serde_json::json!({"text": "你好"})),
        )
        .expect("用户消息落库");
    let host = Host::with_store(Arc::new(SmokeRoot), store);

    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host
        .drive(&base, session, Some("smoke"))
        .await
        .expect("驱动装配成立");

    assert_eq!(report.scope, scope);
    assert_eq!(report.dispatched, 1, "两行债同一轮身份 ⇒ 只派发一次");
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    // 读面从事实源投影全种类：宿主写的用户消息 + 环落的协议事实 + 执行体交回的应答。
    let facts = host.world_facts(scope);
    let kinds: Vec<&str> = facts.iter().map(|fact| fact.kind.as_str()).collect();
    assert!(kinds.contains(&EVENT_USER_MESSAGE), "kinds={kinds:?}");
    assert!(kinds.contains(&"response"), "kinds={kinds:?}");
    let response = facts
        .iter()
        .find(|fact| fact.kind == symbio_plugin_api::EVENT_RESPONSE)
        .expect("应答落库");
    assert_eq!(response.payload["text"], "answer: 你好");
    assert_eq!(response.port, "model", "读面交回端口位");
}
