//! model 端口通用执行面（`DE-048` D-305 裁决一）的用例。

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::round::{ModelUsage, RoundOutput};
use symbio_plugin_api::port::PortFactReader;
use symbio_plugin_api::SymbioKey;

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
                    crate::round::RoundToolCallInfo {
                        id: Some("call-1".into()),
                        wire_id: Some("wire-1".into()),
                        name: Some("echo".into()),
                        arguments: json!({"text": "甲"}),
                        parse_error: None,
                        overflow_notice: None,
                    },
                    crate::round::RoundToolCallInfo {
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
        port: crate::port::MODEL_PORT.to_string(),
        payload,
    }
}

fn exec_context() -> symbio_plugin_api::port::PortExecContext {
    // 开面后的下界：主体随 `AGENT_ID` 载体带进来（发言闸只读这一点，D-339）。
    let meta = serde_json::Map::from_iter([(
        symbio_plugin_api::AGENT_ID.name().to_string(),
        json!("main"),
    )]);
    symbio_plugin_api::port::PortExecContext {
        abort: symbio_plugin_api::ExecAbortSignal::new(),
        idempotency_key: "model-1-1".to_string(),
        sink: symbio_plugin_api::ExecEventSink::Null,
        emit: symbio_plugin_api::port::PortFactEmit::silent(),
        ctx: Some(Arc::new(
            symbio_plugin_api::PluginSimpleRequest::from_parts(json!({}), Some(&meta)),
        )),
        subject: String::new(),
        session_id: "s".to_string(),
        group: String::new(),
        origin: None,
        scratch_dir: None,
        source: String::new(),
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

/// 段预算真的生效：余量只够一段时，低优先级段被裁、高优先级留下（D-243）。
#[test]
fn section_budget_trims_low_priority_sections() {
    use symbio_plugin_api::context::slots;
    use symbio_plugin_api::text::{default_tokenizer, Tokenizer};

    let high = "a".repeat(100);
    let low = "b".repeat(200);
    let tok = default_tokenizer();
    // 预算恰好等于高优先级段的成本：它留下，低优先级那段放不下。
    let budget = tok.count(&high);
    let sections = vec![
        ContextSection::new(slots::READYSET, low, 1),
        ContextSection::new(slots::RECALL, high, 10),
    ];
    let (kept, fit) =
        symbio_plugin_api::context::fit_sections(sections, budget, |text| tok.count(text));

    assert!(fit.trimmed(), "余量不足时必须发生裁剪");
    assert_eq!(kept.len(), 1, "只留得下一段");
    assert_eq!(kept[0].slot, slots::RECALL, "留下的是高优先级段");
    assert!(fit.kept_cost <= fit.budget, "留下段的成本不得超过预算");
}

/// 预算 = 上限 − 历史与固定开销：历史 / 工具定义占得越多，留给段的余量越小。
#[test]
fn section_budget_subtracts_history_and_overhead() {
    let history = vec![PromptMessage {
        role: "user".into(),
        text: "x".repeat(400),
        ..Default::default()
    }];
    let tools = vec![CapabilityMeta {
        name: "echo".into(),
        description: "回显".into(),
        input_schema: json!({"type": "object"}),
        ..Default::default()
    }];

    let empty = section_budget("", &[], &[], 4096);
    let with_history = section_budget("", &[], &history, 4096);
    let with_overhead = section_budget("sys", &tools, &history, 4096);

    assert!(empty > 4000, "没有历史与工具时几乎全部余量给段：{empty}");
    assert!(with_history < empty, "历史消息必须从预算里扣");
    assert!(
        with_overhead < with_history,
        "system prompt 与工具定义必须从预算里扣"
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
        .execute(ctx, input(json!({"root_id": "u-1", "principal": "user"})))
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
    assert_eq!(requested[0].payload["root_id"], "u-1");
    let calls = requested[0].payload["calls"]
        .as_array()
        .expect("载荷是清单");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["id"], "wire-1");
    assert_eq!(calls[0]["name"], "echo");
    assert_eq!(calls[0]["arguments"]["text"], "甲");
    assert_eq!(calls[1]["id"], "wire-2");
}

/// 逐笔标记：N 次调用 ⇒ N 条 `tool.call.requested`，字段是 `(request_id, call_id)`。
///
/// 重试幂等由事实键承担：同一对再次执行时草稿键逐字相同，事实源按 `event_id`
/// 吸收重复——不派生第二笔债。
#[tokio::test]
async fn tool_call_markers_are_one_per_call_and_idempotent() {
    let (ctx, recorder) = recording_context();
    let outcome = executor(FakeOutcome::WithToolCalls)
        .execute(ctx, input(json!({"root_id": "u-1", "principal": "user"})))
        .await;
    assert!(outcome.success());

    let drafts = recorder.drafts.lock().unwrap().clone();
    let markers: Vec<_> = drafts
        .iter()
        .filter(|draft| draft.kind == "tool.call.requested")
        .cloned()
        .collect();
    assert_eq!(markers.len(), 2, "一次调用一条标记（实际草稿: {drafts:?}）");
    assert_eq!(
        markers[0].key,
        "tool.call.requested:tool.requested:model-1-1:wire-1"
    );
    assert_eq!(markers[0].payload["request_id"], "tool.requested:model-1-1");
    assert_eq!(markers[0].payload["call_id"], "wire-1");
    assert_eq!(markers[1].payload["call_id"], "wire-2");

    // 同一 `(request_id, call_id)` 重放：键相同 ⇒ 事实源吸收，只留一条。
    let source = symbio_kernel::store::MemoryStore::<symbio_testkit::fact_source::Fact>::default();
    for draft in markers.iter().chain(markers.iter()) {
        let _ =
            symbio_testkit::fact_source::FactSource::append(&source, "model".into(), draft.clone());
    }
    let facts = symbio_testkit::fact_source::FactSource::facts(&source);
    let marked: Vec<_> = facts
        .iter()
        .filter(|fact| fact.kind == "tool.call.requested")
        .collect();
    assert_eq!(marked.len(), 2, "重放不长出第二条标记（实际: {facts:?}）");
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
    fn facts(&self) -> Vec<symbio_plugin_api::port::PortFact> {
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
        // 收束轮必须有可交付正文：正文与思考都空是失败收束（t77 的判据）。
        Ok(RoundOutput {
            text: "收到".into(),
            ..RoundOutput::default()
        })
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
            input(json!({"provider": "active", "root_id": "u-1", "principal": "user"})),
        )
        .await;

    assert!(outcome.success());
    let (system_prompt, messages, tools) = seen.lock().unwrap().clone().expect("往返已发生");
    assert_eq!(system_prompt, "persona", "system prompt 从事实读");
    assert_eq!(messages, 1, "用户消息从事实读");
    assert_eq!(tools, 0);
}

/// 工具结果已作为 `tool` 节点嵌进请求时，装配**不得**再推一条降级的 assistant 复读。
///
/// t3 的断点：调用身份两套（节点 id / wire id），`already_nested` 比不过 ⇒ 结果被
/// 当成「历史工具结果」降级复读 ⇒ 请求末条变成 assistant ⇒ 按末条判「工具结果轮」
/// 的适配器会把同一笔调用再要一遍（真 provider 同样会被误导）。
#[test]
fn a_nested_tool_result_is_not_degraded_again() {
    let messages = vec![
        super::PromptMessage {
            role: "assistant".into(),
            text: "我先写文件。".into(),
            tool_call_id: None,
            tool: None,
            tool_calls: Some(vec![symbio_plugin_api::PromptToolCall {
                id: "call_w1".into(),
                name: "vdfs_write".into(),
                arguments: serde_json::json!({"path": "notes.md"}),
            }]),
        },
        super::PromptMessage {
            role: "tool".into(),
            text: "{\"ok\":true}".into(),
            tool_call_id: Some("call_w1".into()),
            tool: Some("vdfs_write".into()),
            tool_calls: None,
        },
    ];
    let wire = super::wire_messages(&messages);
    let json = serde_json::to_string(&wire).expect("ChatNode 必然可序列化");
    assert_eq!(
        json.matches("此前调用工具").count(),
        0,
        "已嵌套的工具结果不得再降级成 assistant 复读"
    );
    assert_eq!(wire.len(), 3, "根 + 调用卡 + 结果，各一条：{json}");
    assert_eq!(
        wire.last().unwrap().tool_call_id.as_deref(),
        Some("call_w1"),
        "末条必须是工具结果，且身份与声明逐字相同"
    );
    assert_eq!(
        format!("{:?}", wire.last().unwrap().node_role()),
        "Some(Tool)",
        "末条 role=tool"
    );
}

/// 校准回灌的分母是**未校准**的启发式（`D-433`）。
///
/// 钉的是「用了哪把尺子」而不是具体数值：`default_tokenizer().count` 每条都乘当前
/// 系数再 `+1`，只要有人把分母换成它，这里立刻不等——而且与全局系数取值无关，
/// 所以不依赖其它用例的执行顺序。分母同时含工具定义与逐条结构开销：provider 报的
/// `input` 用量含这两部分，少算就会让观测比值系统性偏大，把系数压下去。
#[test]
fn the_feedback_denominator_is_the_uncalibrated_heuristic() {
    let system = "系统提示：按规范回答";
    let messages = vec![super::PromptMessage {
        role: "user".into(),
        text: "甲乙丙丁".into(),
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }];
    let heuristic = symbio_plugin_api::text::HeuristicTokenizer;
    let expected = heuristic.count(system) + PER_MESSAGE_OVERHEAD + heuristic.count("甲乙丙丁");
    assert_eq!(
        heuristic_prompt_tokens(system, &[], &messages),
        expected,
        "分母必须是原始启发式的和，不能是校准后的 count()"
    );
}
