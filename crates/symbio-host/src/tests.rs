//! 宿主的机制状态：无债即 `idle`，有未还债即撤销。

use super::*;
// 只做链接：环经 creator 注册被这个根构造，测试二进制必须把它的注册项带进来。
use symbio_deploy::PLUGIN_ID_LOOP;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};
use symbio_kernel::store::Store;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::port::{
    port_rows, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::FactDraft;
use symbio_plugin_loop as _;

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
async fn a_session_with_no_debt_is_idle() {
    let host = Host::new(Arc::new(EmptyRoot));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host
        .drive(&base, "s-1", None, ExecEventSink::silent())
        .await
        .expect("驱动装配成立");

    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    assert_eq!(report.dispatched, 0);
    assert!(host.idle_at().is_some(), "无未还债 ⇒ 落 idle");
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
            .map(|reader| reader.facts())
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
        });
        let mut rows = vec![DebtSpec::new(
            "model-reply",
            "model",
            format!("model:{user_id}"),
            payload.clone(),
        )
        .when(EVENT_USER_MESSAGE)
        .unless_id(
            EVENT_CHAT_ASSISTANT_FINAL,
            format!("chat.assistant.final:{user_id}"),
        )];
        if let Some(tool_id) = facts.latest_fact_id(symbio_plugin_api::events::EVENT_ARTIFACT_ADDED)
        {
            rows.push(
                DebtSpec::new(
                    "model-after-tools",
                    "model",
                    format!("model:after:{tool_id}"),
                    payload,
                )
                .when(symbio_plugin_api::events::EVENT_TOOL_REQUESTED)
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

    // 世界事实由写方（会话侧扮演）落进事实源：宿主没有写口。
    let store = Arc::new(symbio_kernel::store::MemoryStore::<Event>::new());
    Store::append(
        &*store,
        Event::pending(
            "u-1",
            EVENT_USER_MESSAGE,
            Entity::Scope,
            Verb::Opened,
            "user",
        )
        .with_payload(serde_json::json!({"text": "你好"})),
    )
    .expect("用户消息落库");
    let host = Host::with_store(Arc::new(SmokeRoot), store);

    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host
        .drive(&base, session, Some("smoke"), ExecEventSink::silent())
        .await
        .expect("驱动装配成立");

    assert_eq!(report.dispatched, 1, "两行债同一轮身份 ⇒ 只派发一次");
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    // 读面从事实源投影全种类：宿主写的用户消息 + 环落的协议事实 + 执行体交回的应答。
    let facts = host.world_facts();
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

// ==================== 工具轮 smoke：model → tool → model ====================

use std::sync::atomic::{AtomicUsize, Ordering};

use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;
use symbio_plugin_api::events::EVENT_TOOL_REQUESTED;
use symbio_plugin_api::port::PortErrorClass;

/// 两笔模型债 + 一笔工具债各派了几次。
#[derive(Default)]
struct LoopCounters {
    model: AtomicUsize,
    tool: AtomicUsize,
}

/// 工具轮的模型：没有产物时交 `tool.requested`；产物已在读面时交终答。
struct LoopModel {
    counters: Arc<LoopCounters>,
}

impl PortProvider for LoopModel {
    fn ports(&self) -> &'static [&'static str] {
        &["model"]
    }

    fn provider_id(&self) -> &str {
        "loop-model"
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(LoopModelExecutor {
            counters: Arc::clone(&self.counters),
        }))
    }
}

struct LoopModelExecutor {
    counters: Arc<LoopCounters>,
}

#[async_trait::async_trait]
impl PortExecutor for LoopModelExecutor {
    async fn execute(&self, exec: PortExecContext, _input: PortInput) -> PortOutcome {
        self.counters.model.fetch_add(1, Ordering::SeqCst);
        let facts = exec
            .reader
            .as_ref()
            .map(|reader| reader.facts())
            .unwrap_or_default();
        let user_id = facts
            .iter()
            .rev()
            .find(|fact| fact.kind == EVENT_USER_MESSAGE)
            .map(|fact| fact.id.clone())
            .unwrap_or_default();
        let saw_artifact = facts.iter().any(|fact| fact.kind == EVENT_ARTIFACT_ADDED);
        let mut drafts = vec![FactDraft {
            key: "smoke:model-response".into(),
            kind: symbio_plugin_api::EVENT_RESPONSE.into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({"text": if saw_artifact { "final" } else { "calling tool" }}),
            produced_by: None,
        }];
        if saw_artifact {
            drafts.push(FactDraft {
                key: format!("chat.assistant.final:{user_id}"),
                kind: EVENT_CHAT_ASSISTANT_FINAL.into(),
                entity: Entity::Scope,
                verb: Verb::Closed,
                payload: serde_json::json!({"text": format!("final: {user_id}")}),
                produced_by: None,
            });
        } else {
            drafts.push(FactDraft {
                key: format!("tool.requested:{}", exec.idempotency_key),
                kind: EVENT_TOOL_REQUESTED.into(),
                entity: Entity::Scope,
                verb: Verb::Asserted,
                payload: serde_json::json!({
                    "turn_id": user_id,
                    "calls": [{"id": "c1", "name": "echo", "arguments": {}}],
                }),
                produced_by: None,
            });
        }
        exec.emit.emit(drafts).await;
        PortOutcome::ok()
    }
}

/// 工具端口：清单来自读面（载荷空），逐调用落产物，最后落 `tool.calls.done`。
struct LoopTool {
    counters: Arc<LoopCounters>,
}

impl PortProvider for LoopTool {
    fn ports(&self) -> &'static [&'static str] {
        &["tool"]
    }

    fn provider_id(&self) -> &str {
        "loop-tool"
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(LoopToolExecutor {
            counters: Arc::clone(&self.counters),
        }))
    }
}

struct LoopToolExecutor {
    counters: Arc<LoopCounters>,
}

#[async_trait::async_trait]
impl PortExecutor for LoopToolExecutor {
    async fn execute(&self, exec: PortExecContext, _input: PortInput) -> PortOutcome {
        self.counters.tool.fetch_add(1, Ordering::SeqCst);
        let facts = exec
            .reader
            .as_ref()
            .map(|reader| reader.facts())
            .unwrap_or_default();
        let Some(request) = facts
            .iter()
            .rev()
            .find(|fact| fact.kind == EVENT_TOOL_REQUESTED)
        else {
            return PortOutcome::failed(PortErrorClass::Retryable, "尚无 tool.requested");
        };
        let calls = request
            .payload
            .get("calls")
            .and_then(|calls| calls.as_array())
            .cloned()
            .unwrap_or_default();
        let mut drafts = Vec::new();
        for call in &calls {
            let id = call
                .get("id")
                .and_then(|id| id.as_str())
                .unwrap_or_default();
            drafts.push(FactDraft {
                key: format!("artifact:{id}"),
                kind: EVENT_ARTIFACT_ADDED.into(),
                entity: Entity::Artifact,
                verb: Verb::Asserted,
                payload: serde_json::json!({
                    "tool": "echo",
                    "tool_call_id": id,
                    "text": "done",
                    "success": true,
                }),
                produced_by: None,
            });
        }
        drafts.push(FactDraft {
            key: format!("tool.calls.done:{}", request.id),
            kind: "tool.calls.done".into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({"requested": request.id, "calls": calls.len()}),
            produced_by: None,
        });
        exec.emit.emit(drafts).await;
        PortOutcome::ok()
    }
}

/// 工具债行（与 `plugin-tool` 同形：有事实才欠、id 从事实派生、载荷空）。
struct LoopToolDebts;

impl DebtSource for LoopToolDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(request_id) = facts.latest_fact_id(EVENT_TOOL_REQUESTED) else {
            return Vec::new();
        };
        vec![DebtSpec::new(
            "tool-calls",
            "tool",
            format!("tool:{request_id}"),
            serde_json::json!({}),
        )
        .when(EVENT_TOOL_REQUESTED)]
    }
}

/// 环 + 模型端口 + 工具端口 + 两条债表的根。
struct ToolLoopRoot {
    counters: Arc<LoopCounters>,
}

#[async_trait::async_trait]
impl Plugin for ToolLoopRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("tool-loop-root", "tool-loop-root")
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
        ring.traverse(path, Arc::clone(&ctx)).await?;

        let visitor = ctx
            .get(symbio_plugin_api::CAPABILITY_VISITOR)
            .ok_or_else(|| PluginError::NotFound("能力收集器缺席".into()))?;
        let model: Arc<dyn PortProvider> = Arc::new(LoopModel {
            counters: Arc::clone(&self.counters),
        });
        for row in port_rows(&model) {
            visitor.put(row).await;
        }
        let tool: Arc<dyn PortProvider> = Arc::new(LoopTool {
            counters: Arc::clone(&self.counters),
        });
        for row in port_rows(&tool) {
            visitor.put(row).await;
        }
        visitor
            .put(debt_row("smoke-model", Arc::new(SmokeDebts)))
            .await;
        visitor
            .put(debt_row("smoke-tool", Arc::new(LoopToolDebts)))
            .await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

/// 断言：派了工具债、执行了调用、落了产物，工具轮之后又来一笔模型债，最后收束。
#[tokio::test]
async fn the_tool_round_is_dispatched_and_fed_back_into_the_model() {
    let session = "smoke-tool-1";

    let store = Arc::new(symbio_kernel::store::MemoryStore::<Event>::new());
    Store::append(
        &*store,
        Event::pending(
            "u-1",
            EVENT_USER_MESSAGE,
            Entity::Scope,
            Verb::Opened,
            "user",
        )
        .with_payload(serde_json::json!({"text": "你好"})),
    )
    .expect("用户消息落库");

    let counters = Arc::new(LoopCounters::default());
    let host = Host::with_store(
        Arc::new(ToolLoopRoot {
            counters: Arc::clone(&counters),
        }),
        store,
    );
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let report = host
        .drive(&base, session, Some("smoke"), ExecEventSink::silent())
        .await
        .expect("驱动装配成立");

    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    assert_eq!(report.dispatched, 3, "model → tool → model");
    assert_eq!(
        counters.model.load(Ordering::SeqCst),
        2,
        "工具轮之后又来一笔模型债（续接那轮）"
    );
    assert_eq!(
        counters.tool.load(Ordering::SeqCst),
        1,
        "工具债派且只派一次"
    );

    let facts = host.world_facts();
    let kinds: Vec<&str> = facts.iter().map(|fact| fact.kind.as_str()).collect();
    assert!(kinds.contains(&EVENT_TOOL_REQUESTED), "kinds={kinds:?}");
    assert!(kinds.contains(&EVENT_ARTIFACT_ADDED), "kinds={kinds:?}");
    assert!(kinds.contains(&"tool.calls.done"), "kinds={kinds:?}");
    assert!(
        kinds.contains(&EVENT_CHAT_ASSISTANT_FINAL),
        "kinds={kinds:?}"
    );

    let artifact = facts
        .iter()
        .find(|fact| fact.kind == EVENT_ARTIFACT_ADDED)
        .expect("产物落库");
    assert_eq!(artifact.payload["tool_call_id"], "c1");
    assert_eq!(artifact.payload["text"], "done");

    let done = facts
        .iter()
        .find(|fact| fact.kind == "tool.calls.done")
        .expect("工具轮收尾事实落库");
    assert_eq!(done.payload["calls"], 1);

    let final_fact = facts
        .iter()
        .find(|fact| fact.kind == EVENT_CHAT_ASSISTANT_FINAL)
        .expect("终答落库");
    assert_eq!(final_fact.payload["text"], "final: u-1");
}
