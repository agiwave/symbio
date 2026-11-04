//! model 端口通用执行面（`DE-048` D-305 裁决一）的用例。

use std::sync::Arc;

use serde_json::json;

use super::*;
use symbio_plugin_api::ModelUsage;

/// 一次往返产物的别名：只在这里出现一次内核类型名，用于实现 trait 的返回签名。
type RoundOutput = symbio_plugin_api::TurnOutput;

/// 固定的假 provider：只验证执行体对载荷、错误与产物的处理，不碰传输层。
struct FakeModel {
    id: &'static str,
    outcome: FakeOutcome,
}

#[derive(Clone)]
enum FakeOutcome {
    Success,
    Aborted,
    RateLimited,
}

#[async_trait::async_trait]
impl ModelProvider for FakeModel {
    fn provider_id(&self) -> &str {
        self.id
    }

    fn api_protocol(&self) -> &str {
        "fake"
    }

    fn rate_limit_ms(&self) -> u64 {
        0
    }

    fn max_context_tokens(&self) -> u32 {
        4096
    }

    async fn effective_context_tokens(&self) -> u32 {
        4096
    }

    async fn round_trip(
        &self,
        _system_prompt: &str,
        _messages: &[ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<RoundOutput, PluginError> {
        match self.outcome {
            FakeOutcome::Success => {
                let mut output = RoundOutput::default();
                output.text = "answer".to_string();
                output.reasoning = "think".to_string();
                output.response_id = Some("resp-1".to_string());
                output.response_text_child_id = "text-1".to_string();
                output.reasoning_child_id = "reason-1".to_string();
                output.finish = ModelFinishReason::ToolCalls;
                output.usage = Some(ModelUsage {
                    input: Some(10),
                    output: Some(5),
                });
                Ok(output)
            }
            FakeOutcome::Aborted => Err(PluginError::Aborted),
            FakeOutcome::RateLimited => Err(PluginError::RateLimited("slow down".to_string())),
        }
    }
}

fn executor(outcome: FakeOutcome) -> Arc<dyn PortExecutor> {
    let active: Arc<dyn ModelProvider> = Arc::new(FakeModel {
        id: "active",
        outcome,
    });
    let port = ModelPort::new(active, Vec::new());
    port.executor().expect("model 端口必须交执行面")
}

fn input(payload: serde_json::Value) -> PortInput {
    PortInput {
        port: crate::port::MODEL_PORT,
        payload,
    }
}

fn exec_context() -> symbio_plugin_api::port::PortExecContext {
    symbio_plugin_api::port::PortExecContext {
        deadline_ms: None,
        abort: symbio_plugin_api::ExecAbortSignal::new(),
        idempotency_key: "model-1-1".to_string(),
        sink: symbio_plugin_api::ExecEventSink::Null,
        emit: symbio_plugin_api::port::PortFactEmit::silent(),
        ctx: None,
        subject: 0,
        group: String::new(),
        origin: None,
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    }
}

#[tokio::test]
async fn executor_turns_round_trip_into_one_fact() {
    let outcome = executor(FakeOutcome::Success)
        .execute(
            exec_context(),
            input(json!({
                "provider": "active",
                "system_prompt": "sys",
                "messages": [],
                "tools": [],
                "root_id": "r1",
            })),
        )
        .await;

    assert!(outcome.success());
    assert_eq!(outcome.facts.len(), 1);
    let fact = &outcome.facts[0];
    assert_eq!(fact["text"], "answer");
    assert_eq!(fact["finish"], "tool_calls");
    assert_eq!(fact["usage"]["input"], 10);
}

#[tokio::test]
async fn executor_maps_abort_and_rate_limit_to_generic_classes() {
    let aborted = executor(FakeOutcome::Aborted)
        .execute(exec_context(), input(json!({})))
        .await;
    assert_eq!(
        aborted.error.as_ref().map(|(class, _)| *class),
        Some(PortErrorClass::NonRetryable)
    );
    assert_eq!(aborted.facts[0]["status"], "aborted");

    let limited = executor(FakeOutcome::RateLimited)
        .execute(exec_context(), input(json!({})))
        .await;
    assert_eq!(
        limited.error.as_ref().map(|(class, _)| *class),
        Some(PortErrorClass::Retryable)
    );
}

#[tokio::test]
async fn unreadable_payload_is_a_non_retryable_failure() {
    let outcome = executor(FakeOutcome::Success)
        .execute(exec_context(), input(json!("not an object")))
        .await;
    assert_eq!(
        outcome.error.as_ref().map(|(class, _)| *class),
        Some(PortErrorClass::NonRetryable)
    );
}
