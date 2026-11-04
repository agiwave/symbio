//! model 端口通用执行面（`DE-048` D-305 裁决一）的用例。

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use symbio_plugin_api::port::PortFactReader;
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
    /// 一轮带两个工具调用的往返（`tool.requested` 的写方就是这个出口）。
    WithToolCalls,
    /// 收束轮但 provider 没回报输入用量（不少协议不给 prompt 用量）。
    FinalWithoutUsage,
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
            FakeOutcome::FinalWithoutUsage => {
                let mut output = RoundOutput::default();
                output.text = "answer".to_string();
                output.finish = ModelFinishReason::Stop;
                output.usage = Some(ModelUsage {
                    input: None,
                    output: Some(5),
                });
                Ok(output)
            }
            FakeOutcome::Aborted => Err(PluginError::Aborted),
            FakeOutcome::RateLimited => Err(PluginError::RateLimited("slow down".to_string())),
            FakeOutcome::WithToolCalls => Ok(RoundOutput {
                finish: ModelFinishReason::ToolCalls,
                tool_calls: vec![
                    symbio_plugin_api::TurnToolCallInfo {
                        id: Some("call-1".into()),
                        wire_id: Some("wire-1".into()),
                        name: Some("echo".into()),
                        arguments: json!({"text": "甲"}),
                        parse_error: None,
                        overflow_notice: None,
                    },
                    symbio_plugin_api::TurnToolCallInfo {
                        id: Some("call-2".into()),
                        wire_id: Some("wire-2".into()),
                        name: Some("echo".into()),
                        arguments: json!({"text": "乙"}),
                        parse_error: None,
                        overflow_notice: None,
                    },
                ],
                ..RoundOutput::default()
            }),
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
        reader: None,
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

/// 收束轮把**实测**上下文水位随终答载荷交出——这是 `meta.context` 的唯一生产写方。
#[tokio::test]
async fn a_final_answer_carries_the_measured_context_watermark() {
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
    let fact = &outcome.facts[0];
    assert_eq!(
        fact["context"]["request_tokens"], 10,
        "输入用量取 provider 回报的实测值"
    );
    assert_eq!(
        fact["context"]["limit_tokens"], 4096,
        "上限取 provider 的有效上下文"
    );
    assert!(
        fact["context"].get("compress_tokens").is_none(),
        "trait 不暴露 reserved_tokens ⇒ 不写压缩阈值（不为读数扩 trait）"
    );
}

/// 没测到输入用量 ⇒ 载荷里**没有** `context`（缺数不出行，不写 0）。
#[tokio::test]
async fn a_final_answer_without_measured_usage_omits_the_watermark() {
    let outcome = executor(FakeOutcome::FinalWithoutUsage)
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
    let fact = &outcome.facts[0];
    assert!(fact.get("context").is_none(), "没测到输入用量 ≠ 用量 0");
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

/// 记录执行体经写口交出的草稿（只认种类 / 键 / 载荷）。
#[derive(Default)]
struct RecordingWriter {
    drafts: Mutex<Vec<symbio_plugin_api::FactDraft>>,
}

#[async_trait::async_trait]
impl symbio_plugin_api::port::PortFactWriter for RecordingWriter {
    async fn emit(&self, facts: Vec<symbio_plugin_api::FactDraft>) {
        self.drafts.lock().unwrap().extend(facts);
    }
}

fn recording_context() -> (
    symbio_plugin_api::port::PortExecContext,
    Arc<RecordingWriter>,
) {
    let recorder = Arc::new(RecordingWriter::default());
    let mut ctx = exec_context();
    ctx.emit = symbio_plugin_api::port::PortFactEmit::new(recorder.clone());
    (ctx, recorder)
}

/// 一次一轮一条：两个工具调用的同一轮只写一条 `tool.requested`，载荷是本轮清单。
#[tokio::test]
async fn a_tool_round_writes_one_tool_requested_for_the_whole_turn() {
    let (ctx, recorder) = recording_context();
    let outcome = executor(FakeOutcome::WithToolCalls)
        .execute(
            ctx,
            input(json!({"root_id": "u-1", "principal": "user", "scope": 7})),
        )
        .await;
    assert!(outcome.success());

    let drafts = recorder.drafts.lock().unwrap().clone();
    let requested: Vec<_> = drafts
        .iter()
        .filter(|draft| draft.kind == "tool.requested")
        .collect();
    assert_eq!(
        requested.len(),
        1,
        "一次一轮一条，不是一次调用一条（实际草稿: {drafts:?}）"
    );
    assert_eq!(
        requested[0].key, "tool.requested:model-1-1",
        "键随本轮派发唯一"
    );
    assert_eq!(requested[0].payload["turn_id"], "u-1");
    let calls = requested[0].payload["calls"]
        .as_array()
        .expect("载荷是清单");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["id"], "wire-1");
    assert_eq!(calls[0]["name"], "echo");
    assert_eq!(calls[0]["arguments"]["text"], "甲");
    assert_eq!(calls[1]["id"], "wire-2");
}

/// 收束轮（无工具调用）不写 `tool.requested`：缺这条事实就不欠工具轮。
#[tokio::test]
async fn a_closing_round_writes_no_tool_requested() {
    let (ctx, recorder) = recording_context();
    let outcome = executor(FakeOutcome::Success)
        .execute(ctx, input(json!({"root_id": "u-1"})))
        .await;
    assert!(outcome.success());
    let drafts = recorder.drafts.lock().unwrap().clone();
    assert!(
        drafts.iter().all(|draft| draft.kind != "tool.requested"),
        "收束轮不应写工具请求（实际草稿: {drafts:?}）"
    );
}

/// 事实读面的静态夹具：执行体只按（种类 / 端口 / 载荷）解释，不认识写方。
struct StaticReader(Vec<symbio_plugin_api::port::PortFact>);

impl PortFactReader for StaticReader {
    fn facts(&self, _scope: u64) -> Vec<symbio_plugin_api::port::PortFact> {
        self.0.clone()
    }
}

/// 只记录「执行体从事实组出来的请求」的假 provider。
struct RecordingModel {
    seen: Arc<Mutex<Option<(String, usize, usize)>>>,
}

#[async_trait::async_trait]
impl ModelProvider for RecordingModel {
    fn provider_id(&self) -> &str {
        "active"
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
        system_prompt: &str,
        messages: &[ChatNode],
        tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<RoundOutput, PluginError> {
        *self.seen.lock().unwrap() = Some((system_prompt.to_string(), messages.len(), tools.len()));
        Ok(RoundOutput::default())
    }
}

#[tokio::test]
async fn executor_builds_the_request_from_the_fact_reader() {
    let seen = Arc::new(Mutex::new(None));
    let active: Arc<dyn ModelProvider> = Arc::new(RecordingModel {
        seen: Arc::clone(&seen),
    });
    let executor = ModelPort::new(active, Vec::new())
        .executor()
        .expect("model 端口必须交执行面");

    let facts = vec![
        symbio_plugin_api::port::PortFact {
            id: "u-1".into(),
            port: String::new(),
            kind: "user.message".into(),
            payload: json!({"text": "你好"}),
        },
        symbio_plugin_api::port::PortFact {
            id: "cfg-1".into(),
            port: String::new(),
            kind: "session.config".into(),
            payload: json!({"system_prompt": "persona", "tools": []}),
        },
    ];
    let mut ctx = exec_context();
    ctx.reader = Some(Arc::new(StaticReader(facts)));

    let outcome = executor
        .execute(
            ctx,
            input(json!({"provider": "active", "root_id": "u-1", "principal": "user", "scope": 7})),
        )
        .await;

    assert!(outcome.success());
    let (system_prompt, messages, tools) = seen.lock().unwrap().clone().expect("往返已发生");
    assert_eq!(system_prompt, "persona", "system prompt 从事实读");
    assert_eq!(messages, 1, "用户消息从事实读");
    assert_eq!(tools, 0);
}
