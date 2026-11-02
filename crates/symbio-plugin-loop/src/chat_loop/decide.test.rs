//! `chat_loop/decide.rs` 的缺席判据（D-298）。
//!
//! `port.model` 0 行（没装 model 插件 / 停用 / 收集没跑）时，判决往返的收法：
//! 落 `request` + `failed` 两格后**按无判决中止**——不是 `Escalate{unclassified}`
//! （那是「判完了，判成升格」），也不是「本轮当作没打过模型」的静默路径
//! （账上说得清这次往返想要、没打成）。

use std::sync::Arc;

use super::super::super::test_support::{
    default_settings, loop_session, turn_spec, FakeSessionPort,
};
use super::super::state::{SessionContext, TurnState};
use symbio_kernel::dialog::DecisionPlan;
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};
use symbio_kernel::store::{EventStore, Store};
use symbio_plugin_api::PluginInvokeRequestExt;

/// 什么都不提供的插件：缺席路径在派发之前就返回，`parent` 不会被问到
/// （断言它收不到任何调用是同一条判据的另一半——这里不需要，路径先于它返回）。
struct NopPlugin;

#[async_trait::async_trait]
impl Plugin for NopPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("nop", "nop")
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(format!(
            "nop 不路由：{}",
            ctx.get(symbio_sdk::keys::PATH).unwrap_or_default()
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(format!("nop 不遍历：{path}")))
    }
}

/// 缺席收法：两格账 + 无判决中止，不出现 `Escalate`，也不出现静默路径。
#[tokio::test]
async fn a_missing_model_port_lands_a_failed_fact_and_aborts_the_round() {
    let facts = Arc::new(EventStore::new());
    let port = Arc::new(FakeSessionPort::new(Vec::new()).with_facts(facts.clone()));
    let spec = turn_spec(
        "s-decide-absent",
        std::env::temp_dir(),
        std::env::temp_dir(),
        default_settings(),
    );
    let session = Arc::new(loop_session(spec, facts.clone(), port));
    let context = SessionContext {
        messages: Vec::new(),
        session,
        consolidated: None,
    };
    let turn = TurnState {
        turn_no: 3,
        user_seq: 42,
        drive_seq: 7,
        request_round: 1,
        ..Default::default()
    };

    // ctx 没有能力注册面 ⇒ `port.model` 的读数是 0 行（缺席，不是故障）。
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);

    let decision = super::decide_by_model(
        &ctx,
        &context,
        &turn,
        &parent,
        "s".to_string(),
        DecisionPlan {
            system_prompt: "判".to_string(),
            messages: Vec::new(),
            model_id: None,
        },
    )
    .await;

    assert_eq!(
        decision.verdict, None,
        "缺席按无判决中止——`Escalate` 是「判成了升格」，不是缺席的替身"
    );
    assert_eq!(decision.model_calls, 0, "模型一次都没打");

    let all = facts.range(Seq::new(0));
    let of_call = |e: &Event| e.payload.get("id").and_then(|v| v.as_str()) == Some("model-3-d7-c1");
    let request = all
        .iter()
        .find(|e| e.kind == "request" && of_call(e))
        .expect("request 格在（这笔往返想要）");
    assert_eq!(request.payload["port"], "model");
    assert_eq!(request.payload["via"], "classify/decide", "身份随账走");
    assert_eq!(request.turn, 3, "账属于本轮");
    assert_eq!(request.produced_by, Some(42), "溯源指向本轮用户格");

    let failed = all
        .iter()
        .find(|e| e.kind == "failed" && of_call(e))
        .expect("failed 格在（没打成要说得清）");
    assert!(
        failed.payload["why"]
            .as_str()
            .expect("失败格带 why")
            .contains("缺席"),
        "失败原因点名缺席：{}",
        failed.payload["why"]
    );

    // 静默路径不存在：这笔账没有 `sent` / `response`——没派发过就不得有派发格。
    assert!(
        !all.iter()
            .any(|e| matches!(e.kind.as_str(), "sent" | "response") && of_call(e)),
        "缺席发生在派发之前：不得出现 sent / response 格"
    );
}
