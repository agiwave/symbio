//! `policy.classify` 缺模型端口的失败通道（D-298）。
//!
//! 判决往返需要一个模型提供者：`port.model` 缺席时**不许静默滑进工具循环**。
//! 本文件钉两条事实：失败格 `turn.stage{stage:"classify",result:"failed"}` 入格，
//! 以及本轮以错误交回（编排层据此中止本轮）；同时 `abort_round_trip` 落的
//! `request` / `failed` 两格在日志上说得清「这次往返想要、没打成」。

use std::sync::Arc;

use symbio_kernel::dialog::{DecideOutcome, DecisionPlan};
use symbio_kernel::event::{Entity, Event, Seq, Verb};
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};
use symbio_kernel::store::EventStore;
use symbio_kernel::store::Store;

use super::classify;
use crate::chat_loop::state::{ChatOrchestrator, SessionContext, TurnState};
use crate::control_decl::EVENT_TURN_STAGE;
use crate::test_support::{default_settings, session_port_of, turn_spec, FakeSessionPort};

/// 判决插件替身：不判规则，一律要一次模型往返（`NeedsModel`）。
///
/// 本轮要问的是「要往返但端口缺席」那一刻的失败通道，不是判决器自己怎么判。
struct NeedsModelDecider;

#[async_trait::async_trait]
impl Plugin for NeedsModelDecider {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("decide-needs-model", "NeedsModelDecider")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let outcome = DecideOutcome::NeedsModel {
            plan: DecisionPlan {
                system_prompt: String::new(),
                messages: Vec::new(),
                model_id: None,
            },
        };
        let value = serde_json::to_value(outcome).expect("判决契约可序列化");
        Ok(PluginPayload::new(&value))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(
            "needs-model-decider has no vdfs".into(),
        ))
    }
}

/// 编排器有的那份转写写口；本用例不测压缩改道，交一个丢弃口。
struct DiscardWriter;

#[async_trait::async_trait]
impl symbio_kernel::exec::ExecTranscriptWriter for DiscardWriter {
    async fn apply(&self, _m: symbio_sdk::chat::chat_node::ChatNode) {}
}

/// 缺 `port.model` ⇒ 落失败事实并中止本轮（D-298）。
#[tokio::test]
async fn a_missing_model_port_lands_a_failed_stage_and_stops_the_turn() {
    let raw = Arc::new(EventStore::new());
    let user_seq = raw
        .append(
            Event::pending(
                "u-1",
                symbio_plugin_api::events::EVENT_USER_MESSAGE,
                Entity::Message,
                Verb::Asserted,
                1,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "你好" })),
        )
        .expect("用户格入格");
    let facts: Arc<dyn Store<Event = Event>> = raw.clone();

    let port = Arc::new(FakeSessionPort::new(Vec::new()).with_facts(raw.clone()));
    let session = session_port_of(port);
    let spec = turn_spec(
        "policy-classify",
        std::env::temp_dir().join("symbio-test/tool-archives"),
        std::env::temp_dir().join("symbio-test/transcripts"),
        default_settings(),
    );
    let view = spec.view();

    let orchestrator = ChatOrchestrator {
        parent: Some(Arc::new(NeedsModelDecider) as Arc<dyn Plugin>),
        context_limit: 8192,
        view: view.clone(),
        facts: Some(facts.clone()),
        session: session.clone(),
        writer: Arc::new(DiscardWriter),
        compose_enabled: false,
        opening_enabled: false,
        presentation_tree: symbio_sdk::turn::node_build::PresentationTree::Grouped,
    };
    let mut context = SessionContext {
        messages: Vec::new(),
        view,
        facts: Some(facts.clone()),
        session,
        consolidated: None,
    };
    let mut state = TurnState {
        turn_no: 1,
        user_seq: user_seq.value(),
        drive_seq: user_seq.value(),
        now_ms: 1,
        ..Default::default()
    };
    state.facts = raw.range(Seq::new(0));

    // 没有 `CAPABILITY_VISITOR` ⇒ `port.model` 读不到提供者；判决插件照常要往返。
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));

    let err = classify(&orchestrator, &ctx, &facts, &mut context, &mut state)
        .await
        .expect_err("缺模型端口必须中止本轮");
    match err {
        crate::turn_exec::ActorError::Read(e) => {
            assert!(
                format!("{e:?}").contains("port.model"),
                "中止原因要说得出是端口缺席（实得 {e:?}）"
            );
        }
        crate::turn_exec::ActorError::Append(e) => {
            panic!("本用例不该走到落格失败：{e:?}")
        }
    }

    let all = raw.range(Seq::new(0));
    let stage = all
        .iter()
        .find(|e| e.kind == EVENT_TURN_STAGE && e.payload["stage"] == "decide")
        .expect("判决阶段格必须入格");
    assert_eq!(
        stage.payload["result"], "failed",
        "缺端口是失败，不是「无判决」——静默滑进工具循环正是 D-298 否掉的那条路径"
    );
    assert!(
        all.iter().any(|e| e
            .payload
            .get("why")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|w| w.contains("port.model"))),
        "abort_round_trip 的失败格要说得出这次往返为什么没打成"
    );
}
