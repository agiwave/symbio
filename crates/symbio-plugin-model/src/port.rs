//! model 端口的世界提供者（D-298 M1）：解析与适配器构造收在 model 插件一侧。
//!
//! 环（loop）只按端口名取——provider 怎么解析（严格查 `preferred` → 落回当次
//! 生效者）、适配器怎么构造，都是这里的私事。`ModelPort` 在 traverse 里随能力
//! 目录一起装：生效者缺席 ⇒ 不注册端口行 ⇒ 环在调用点响亮失败（落失败事实
//! 并中止），而不是在这里猜一个别的模型。

use std::sync::{Arc, Mutex};

use symbio_plugin_api::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, CHAT_STYLE_TAG_RUN, CHAT_STYLE_TAG_TOOL,
};
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortInput, PortOutcome,
    PortProvider, PortReply,
};
use symbio_plugin_api::ports;
use symbio_plugin_api::{
    llm_short_id, CapabilityMeta, CapabilityRow, Entity, ExecAbortSignal, ExecEnv, ExecEventSink,
    ExecTranscriptWriter, FactDraft, LlmAdapter, ModelFinishReason, ModelProvider, PluginError,
    PromptMessage, ProviderLlmAdapter, TurnOutput, TurnToolCallInfo, Verb, EVENT_FAILED,
    EVENT_RESPONSE,
};

/// `port.model` 的提供者：当次生效者 + 完整目录（严格解析用）。
///
/// 目录 = 全部已启用的 provider（与能力目录同一批 `BoundProvider`）；生效者
/// 必然是目录一员（不在 ⇒ traverse 不注册本行，见模块文档）。`preferred`
/// 查目录不中生 ⇒ 落回生效者——降级方向只有这一处定义。
pub(crate) struct ModelPort {
    active: Arc<dyn ModelProvider>,
    catalog: Vec<Arc<dyn ModelProvider>>,
}

impl ModelPort {
    pub(crate) fn new(
        active: Arc<dyn ModelProvider>,
        catalog: Vec<Arc<dyn ModelProvider>>,
    ) -> Self {
        Self { active, catalog }
    }

    /// 注册行（本提供者只服务一个端口，行数恒为 1）。
    pub(crate) fn rows(self: Arc<Self>) -> Vec<CapabilityRow> {
        symbio_plugin_api::port::port_rows(&(self as Arc<dyn PortProvider>))
    }
}

impl PortProvider for ModelPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ports::MODEL]
    }

    fn provider_id(&self) -> &str {
        self.active.provider_id()
    }

    fn llm_round_trip(
        &self,
        preferred: Option<&str>,
        system_prompt: &str,
        abort: ExecAbortSignal,
    ) -> Option<Arc<dyn LlmAdapter>> {
        let provider = preferred
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        Some(Arc::new(ProviderLlmAdapter::for_turn(
            provider.clone(),
            system_prompt,
            abort,
        )))
    }

    fn model(&self) -> Option<Arc<dyn ModelProvider>> {
        Some(self.active.clone())
    }

    /// 通用执行面（`DE-048` D-305 裁决一）：一次往返的请求构造、流式帧、工具调用
    /// 累计、用量与计价、终止判定都在执行体里；环只把 `request` 载荷原样交进来。
    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(ModelExecutor {
            active: self.active.clone(),
            catalog: self.catalog.clone(),
        }))
    }
}

/// `request(port=model)` 的载荷（行契约由 model 插件自己持有，环不认识这些键）。
#[derive(Debug, serde::Deserialize)]
struct RoundTripRequest {
    /// 首选 provider id；不在目录 ⇒ 落回当次生效者（与 `llm_round_trip` 同一条规则）。
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    messages: Vec<PromptMessage>,
    #[serde(default)]
    tools: Vec<CapabilityMeta>,
    #[serde(default)]
    root_id: String,
    /// 记账与装配用的容器事实（`D-314`）：`response` 载荷的 `turn_id` / `principal`、
    /// `calls` 的 `call_key` 都从这里取，环不再代装配。
    #[serde(default)]
    principal: String,
    #[serde(default)]
    turn: u64,
    #[serde(default)]
    drive: u64,
    /// 这一笔债的逻辑 id（与 `sent` / `response` 的幂等键同源）。
    #[serde(default)]
    round_id: String,
    /// 工具轮序号（环的纯值；只用于应答承载里合成 id 的跨轮唯一性）。
    #[serde(default)]
    round: usize,
}

/// 一次模型往返的执行体。
struct ModelExecutor {
    active: Arc<dyn ModelProvider>,
    catalog: Vec<Arc<dyn ModelProvider>>,
}

#[async_trait::async_trait]
impl PortExecutor for ModelExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let request: RoundTripRequest = match serde_json::from_value(input.payload) {
            Ok(request) => request,
            Err(error) => {
                let why = format!("模型请求载荷不可读：{error}");
                // 连载荷都读不出来，这一笔也必须收口（`failed` 由执行体落格，`D-314`）。
                emit_outcome(
                    &exec.emit,
                    EVENT_FAILED,
                    serde_json::json!({ "status": "failed", "why": &why }),
                )
                .await;
                return PortOutcome::failed(PortErrorClass::NonRetryable, why);
            }
        };
        let provider = request
            .provider
            .as_deref()
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        // 请求构造归执行体（D-305 裁决一 / D-306 裁决二）：环交进的是**值**
        // （`PromptMessage`），线格式在这里收口。
        let wire = wire_messages(&request.messages);
        let started = std::time::Instant::now();
        // 流式增量是**瞬时帧**，不进事实表（`D-314` 两条通道）：provider 往 `env.sink()`
        // 写帧，这一出口记下节点 id 与半截正文（供 `response` / `failed` 载荷使用），
        // 再原样转发给展示出口。丢帧不影响正确性——终稿以 `response` 为准。
        let rows = Arc::new(StreamRows::new(exec.sink.clone()));
        let env = ExecEnv::new(ExecEventSink::direct(rows.clone()), exec.abort.clone());
        let outcome = match provider
            // internal-model-allow IM-001/gate: 端口执行体转发欠债行给的工具集（D-305 裁决一）。
            .round_trip(
                &request.system_prompt,
                &wire,
                &request.tools,
                &request.root_id,
                &env,
            )
            .await
        {
            Ok(output) => PortOutcome {
                // 一次往返的终稿 → `response` 载荷：正文 / 思考、节点身份、终答折叠、
                // 用量与工具调用集合展开都由执行体装配（`D-314`）。
                facts: vec![outcome_payload(
                    &request,
                    Some(&output),
                    &rows.snapshot(),
                    started,
                    None,
                )],
                error: None,
                // 应答承载（D-317）：assistant 消息的组装是执行体的私事，环只追加与配对。
                reply: prompt_reply(&request, &output),
            },
            Err(PluginError::Aborted) => PortOutcome {
                // 半截产出仍要留痕：中止是这一笔的状态，不是「没有发生过」。
                facts: vec![outcome_payload(
                    &request,
                    None,
                    &rows.snapshot(),
                    started,
                    Some(("aborted", "aborted")),
                )],
                error: Some((PortErrorClass::NonRetryable, "aborted".to_string())),
                ..Default::default()
            },
            Err(PluginError::RateLimited(why)) => PortOutcome {
                facts: vec![outcome_payload(
                    &request,
                    None,
                    &rows.snapshot(),
                    started,
                    Some(("failed", &why)),
                )],
                error: Some((PortErrorClass::Retryable, why)),
                ..Default::default()
            },
            Err(error) => {
                let why = error.to_string();
                PortOutcome {
                    facts: vec![outcome_payload(
                        &request,
                        None,
                        &rows.snapshot(),
                        started,
                        Some(("failed", &why)),
                    )],
                    error: Some((PortErrorClass::NonRetryable, why)),
                    ..Default::default()
                }
            }
        };
        // 账：`response` / `failed` 由执行体经宿主写口落格（`D-314`）；
        // 键与信封（`port` / `id` / `attempt` / `via`）由宿主按本笔债身份盖章。
        if let Some(payload) = outcome.facts.first() {
            let kind = if outcome.error.is_none() {
                EVENT_RESPONSE
            } else {
                EVENT_FAILED
            };
            emit_outcome(&exec.emit, kind, payload.clone()).await;
        }
        outcome
    }
}

/// 执行体落协议事实的统一出口（`D-314`）：只交载荷，键与信封由宿主盖章。
///
/// `key` 只是无信封宿主的兜底——模型往返的宿主写口带着本笔债身份，
/// 会把 `response` / `failed` 的键改写成 `<kind>-<port>-<id>-<attempt>`。
async fn emit_outcome(emit: &PortFactEmit, kind: &str, payload: serde_json::Value) {
    emit.emit(vec![FactDraft {
        key: format!("{kind}-outcome"),
        kind: kind.to_string(),
        entity: Entity::Scope,
        verb: Verb::Asserted,
        payload,
        produced_by: None,
    }])
    .await;
}

/// 一次往返写进 `response` / `failed` 的载荷（行契约归本插件，`D-314`）。
///
/// 键名与环侧 `output_fields` 同义：`text` / `text_id` / `reasoning` / `reasoning_id` /
/// `answer` / `answer_id` / `truncated` / `principal` / `turn_id`，外加本轮的工具调用
/// 集合展开（`tool_calls` 是给环折 `LlmTurn` 的原始值，`calls` 是派发侧读的调用键）。
fn outcome_payload(
    request: &RoundTripRequest,
    output: Option<&TurnOutput>,
    stream: &serde_json::Value,
    started: std::time::Instant,
    stopped: Option<(&str, &str)>,
) -> serde_json::Value {
    let of = |key: &str| {
        stream
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    // 终稿优先取 provider 交回的全文；中止 / 未流式时落回执行体自己观测到的半截。
    let text = match output {
        Some(o) if !o.text.trim().is_empty() => o.text.clone(),
        _ => of("text"),
    };
    let reasoning = match output {
        Some(o) if !o.reasoning.trim().is_empty() => o.reasoning.clone(),
        _ => of("reasoning"),
    };
    // 节点身份由执行体持有：观测到的帧 id 优先，其次 provider 声明的子节点 id，
    // 都没有才按这一笔债的逻辑 id 合成（随事实入格，读侧不再猜）。
    let stream_id = |key: &str| stream.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let text_id = stream_id("text_id")
        .or_else(|| {
            output
                .map(|o| o.response_text_child_id.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| (!text.trim().is_empty()).then(|| format!("{}-text", request.round_id)));
    let reasoning_id = stream_id("reasoning_id")
        .or_else(|| {
            output
                .map(|o| o.reasoning_child_id.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            (!reasoning.trim().is_empty()).then(|| format!("{}-reasoning", request.round_id))
        });
    // 终答折叠归执行体（`D-314`）：有工具调用的轮次交付正文；收束轮按
    // 正文 → 截断则空 → 端口思考 → 流上思考 的顺序取值；失败轮交付半截正文。
    let answer = match output {
        Some(o) if !o.tool_calls.is_empty() => text.clone(),
        Some(o) => fold_answer(o, &reasoning),
        None => text.clone(),
    };
    let answer_id = symbio_plugin_api::event_output_text_node_id(
        &answer,
        &reasoning,
        text_id.as_deref(),
        reasoning_id.as_deref(),
        Some(&request.root_id),
    );
    let mut payload = serde_json::json!({
        "turn_id": request.root_id,
        "text": text,
        "text_id": text_id,
        "reasoning": reasoning,
        "reasoning_id": reasoning_id,
        "answer": answer,
        "answer_id": answer_id,
        "truncated": output.is_some_and(|o| o.finish.is_length()),
        "principal": request.principal,
        "cost_ms": started.elapsed().as_millis() as u64,
    });
    if let Some(o) = output {
        payload["response_id"] = serde_json::json!(o.response_id);
        payload["finish"] = serde_json::json!(finish_str(&o.finish));
        payload["decision"] = serde_json::json!(if o.tool_calls.is_empty() {
            "halt"
        } else {
            "tools"
        });
        payload["tool_calls"] = serde_json::json!(o
            .tool_calls
            .iter()
            .map(|call| serde_json::json!({
                "id": call.id,
                "wire_id": call.wire_id,
                "name": call.name,
                "arguments": call.arguments,
                "parse_error": call.parse_error,
                "overflow_notice": call.overflow_notice,
            }))
            .collect::<Vec<_>>());
        payload["calls"] = serde_json::json!(dispatch_calls(request, &o.tool_calls));
        // 实测用量：只写测到的那一侧；缺席不写 `usage: null`（与环侧 `output_fields`
        // 同一口径，`token_ledger` 按 `measured` 数条数）。
        if let Some(usage) = o.usage.as_ref() {
            if usage.input.is_some() || usage.output.is_some() {
                let mut measured = serde_json::Map::new();
                if let Some(v) = usage.input {
                    measured.insert("input".into(), serde_json::json!(v));
                }
                if let Some(v) = usage.output {
                    measured.insert("output".into(), serde_json::json!(v));
                }
                payload["usage"] = serde_json::Value::Object(measured);
            }
        }
        if let Some(spend) = o.spend.as_ref() {
            payload["spend"] = serde_json::json!({
                "amount_nano": spend.amount_nano,
                "currency": spend.currency,
            });
        }
    }
    if let Some((status, why)) = stopped {
        // 停法进载荷：重建按它定节点状态，而 `failed` 本身不区分「用户按停」
        // 与「执行体报错」——那条区分只在这一格。
        payload["status"] = serde_json::json!(status);
        payload["why"] = serde_json::json!(why);
    }
    payload
}

/// 末轮答话的取值（`final_answer` 迁入执行体，`D-314`）：正文 → 截断则空 →
/// 端口思考 → 流上思考。四条顺序都是判据，不是偏好。
fn fold_answer(output: &TurnOutput, stream_reasoning: &str) -> String {
    if !output.text.trim().is_empty() {
        output.text.clone()
    } else if output.finish.is_length() {
        String::new()
    } else if !output.reasoning.trim().is_empty() {
        output.reasoning.clone()
    } else {
        stream_reasoning.to_string()
    }
}

/// 本轮工具调用 → 派发侧读的调用键（`response.calls`）。
///
/// 格式与环侧 `tool_dispatch::call_key` 逐字相同：`call_key` 的写方从此是执行体，
/// 派发侧只读这份事实（缺失时才回退到旧式重算）。
fn dispatch_calls(
    request: &RoundTripRequest,
    tool_calls: &[TurnToolCallInfo],
) -> Vec<serde_json::Value> {
    tool_calls
        .iter()
        .enumerate()
        .map(|(idx, call)| {
            let mut id = format!(
                "{}-{}-d{}-{}-c{idx:03}",
                ports::TOOL,
                request.turn,
                request.drive,
                request.round_id
            );
            if let Some(wire) = call.id.as_deref().filter(|s| !s.trim().is_empty()) {
                id.push('-');
                id.push_str(wire);
            }
            serde_json::json!({
                "port": ports::TOOL,
                "id": id,
                "name": call.name.clone().unwrap_or_default(),
                "arguments": call.arguments.clone(),
            })
        })
        .collect()
}

/// 一轮往返的**应答承载**（D-317）：assistant 正文 + 逐条 `tool_calls` 声明。
///
/// 这一段原本住在环里（`turn_runner::facts::assistant_exchange`）；迁到这里之后，
/// 环只把 [`PortReply::messages`] 按序追加进下一轮请求，不再构造任何世界消息。
///
/// id 的取法与环侧旧实现**逐字同源**（wire id → 节点 id → 合成），跨轮唯一性由
/// `round` 保证；`links` 交回「节点 id → 应答配对键」，环按它把工具结果挂回调用。
fn prompt_reply(request: &RoundTripRequest, output: &TurnOutput) -> PortReply {
    let mut messages: Vec<PromptMessage> = Vec::new();
    let mut links = Vec::new();
    if !output.text.trim().is_empty() {
        messages.push(PromptMessage {
            role: "assistant".into(),
            text: output.text.trim().to_string(),
            tool_call_id: None,
            tool: None,
            tool_calls: None,
        });
    }
    for (index, call) in output.tool_calls.iter().enumerate() {
        let name = call.name.as_deref().unwrap_or("<unnamed>");
        let id = prompt_call_id(call, request.round, index);
        if let Some(key) = call.id.clone().filter(|s| !s.trim().is_empty()) {
            links.push((key, id.clone()));
        }
        messages.push(PromptMessage {
            role: "assistant".into(),
            text: format!("调用工具 {} {}", name, call.arguments),
            tool_call_id: Some(id.clone()),
            tool: Some(name.to_string()),
            tool_calls: Some(vec![symbio_plugin_api::PromptToolCall {
                id,
                name: name.to_string(),
                arguments: call.arguments.clone(),
            }]),
        });
    }
    PortReply { messages, links }
}

/// 合成 / 选取 `tool_call_id`（跨轮唯一，见 [`prompt_reply`]）。
///
/// 优先级与环侧旧实现同一口径：wire id → 节点 id → 合成 `call_<tool>_<轮>_<序号>`。
fn prompt_call_id(call: &TurnToolCallInfo, round: usize, index: usize) -> String {
    fn present(id: &Option<String>) -> Option<String> {
        id.clone().filter(|s| !s.trim().is_empty())
    }
    present(&call.wire_id)
        .or_else(|| present(&call.id))
        .unwrap_or_else(|| {
            format!(
                "call_{}_{}_{}",
                call.name.as_deref().unwrap_or("<unnamed>"),
                round,
                index
            )
        })
}

/// 执行体的流式读数与转发口（`D-314` 的「流」通道）。
///
/// provider 写出的每一帧是**瞬时帧**：本口记下正文 / 推理的节点 id 与（可能半截的）
/// 正文，然后原样转发给展示出口——不进事实表、不参与重放、不写幂等键。记下的读数
/// 随 `response` / `failed` 载荷交出，环不再从 `StreamNodeFacts` 桥取这一份账。
struct StreamRows {
    display: ExecEventSink,
    state: Mutex<StreamState>,
}

#[derive(Default)]
struct StreamState {
    text_id: Option<String>,
    text: String,
    reasoning_id: Option<String>,
    reasoning: String,
}

impl StreamRows {
    fn new(display: ExecEventSink) -> Self {
        Self {
            display,
            state: Mutex::new(StreamState::default()),
        }
    }

    /// 一次读、两处共用：`response` 载荷里的读数与中止时的半截正文。
    fn snapshot(&self) -> serde_json::Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::json!({
            "text": state.text,
            "reasoning": state.reasoning,
            "text_id": state.text_id,
            "reasoning_id": state.reasoning_id,
        })
    }

    /// 按帧形状记账：快照（`content`）替换正文并登记 id，增量（`delta`）按 id
    /// 对上正文或推理。工具骨架帧不进这两格。
    ///
    /// 增量帧**不带 `msg_type`**（`llm_emit_delta` 只写 `id` + `delta`），因此
    /// 不能照快照那样按类型分派——否则推理增量会被记进正文格。判据改成「谁登记
    /// 过这个 id」：快照先建身份，增量按 id 找到它那一路。
    fn observe(&self, node: &ChatNode) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let StreamState {
            text_id,
            text,
            reasoning_id,
            reasoning,
        } = &mut *state;
        if let Some(delta) = node.delta.as_deref() {
            if reasoning_id.as_deref() == Some(node.id.as_str()) {
                reasoning.push_str(delta);
            } else if text_id.as_deref() == Some(node.id.as_str()) {
                text.push_str(delta);
            }
            return;
        }
        match node.msg_type {
            Some(NodeKind::Notice) => Self::snapshot_into(reasoning_id, reasoning, node),
            Some(NodeKind::Skeleton) => {}
            _ => Self::snapshot_into(text_id, text, node),
        }
    }

    fn snapshot_into(id: &mut Option<String>, body: &mut String, node: &ChatNode) {
        if let Some(MessageContent::Text(text)) = node.content.as_ref() {
            *id = Some(node.id.clone());
            *body = text.clone();
        }
    }
}

#[async_trait::async_trait]
impl ExecTranscriptWriter for StreamRows {
    async fn apply(&self, message: ChatNode) {
        self.observe(&message);
        self.display.emit(message).await;
    }
}

/// 把最小消息形态翻成 provider 线格式（`D-305` 裁决一：请求构造归执行体）。
///
/// 规则与 `ProviderLlmAdapter::generate_turn` 的线格式段落逐条同义：
/// 同一次工具调用只发**最终**那条结果；本次请求里声明过的调用用树形骨架
/// （run 根 + tool 子节点），跨轮结果降级成 assistant 文本，其余按角色直译。
fn wire_messages(messages: &[PromptMessage]) -> Vec<ChatNode> {
    // 同一次调用只把**最终**那条结果发给模型（协议侧「一次调用一条结果」）。
    let last_result_of: Vec<(String, usize)> = {
        let mut last: Vec<(String, usize)> = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            if m.role != "tool" {
                continue;
            }
            if let Some(id) = &m.tool_call_id {
                match last.iter_mut().find(|(k, _)| k == id) {
                    Some(slot) => slot.1 = i,
                    None => last.push((id.clone(), i)),
                }
            }
        }
        last
    };

    let declared_call_ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flat_map(|cs| cs.iter().map(|c| c.id.clone()))
        .collect();

    let mut wire: Vec<ChatNode> = Vec::with_capacity(messages.len() * 2);
    for (idx, m) in messages.iter().enumerate() {
        if m.role == "tool"
            && m.tool_call_id.as_ref().is_some_and(|id| {
                last_result_of
                    .iter()
                    .find(|(k, _)| k == id)
                    .is_some_and(|(_, at)| *at != idx)
            })
        {
            continue;
        }
        match (&m.role[..], &m.tool_calls) {
            ("assistant", Some(calls)) if !calls.is_empty() => {
                let root_id = llm_short_id();
                let root = ChatNode {
                    id: root_id.clone(),
                    role: Some(NodeRole::Assistant),
                    msg_type: Some(NodeKind::Skeleton),
                    content: Some(MessageContent::Text(m.text.clone())),
                    ..Default::default()
                }
                .with_style_tag(CHAT_STYLE_TAG_RUN);
                let mut nodes: Vec<ChatNode> = vec![root];
                for c in calls {
                    nodes.push(
                        ChatNode {
                            id: llm_short_id(),
                            parent_id: Some(root_id.clone()),
                            role: Some(NodeRole::Assistant),
                            msg_type: Some(NodeKind::Skeleton),
                            // 参数走 content（展平时从这里 re-parse）。
                            content: Some(MessageContent::Text(c.arguments.to_string())),
                            tool_call_id: Some(c.id.clone()),
                            ..Default::default()
                        }
                        .with_style_tag(CHAT_STYLE_TAG_TOOL)
                        .with_tool_name(Some(c.name.clone())),
                    );
                }
                for r in messages
                    .iter()
                    .enumerate()
                    .filter(|(_, x)| x.role == "tool")
                    .filter(|(_, x)| calls.iter().any(|c| Some(&c.id) == x.tool_call_id.as_ref()))
                    .filter(|(at, x)| {
                        x.tool_call_id.as_ref().is_some_and(|id| {
                            last_result_of
                                .iter()
                                .find(|(k, _)| k == id)
                                .is_some_and(|(_, last)| last == at)
                        })
                    })
                    .map(|(_, r)| r)
                {
                    nodes.push(ChatNode {
                        id: llm_short_id(),
                        parent_id: nodes
                            .iter()
                            .find(|n| n.tool_call_id == r.tool_call_id)
                            .map(|n| n.id.clone()),
                        role: Some(NodeRole::Tool),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(r.text.clone())),
                        tool_call_id: r.tool_call_id.clone(),
                        ..Default::default()
                    });
                }
                wire.extend(nodes);
            }
            // 历史工具结果（跨轮）：降级成 assistant 文本（协议要求 tool_call_id 必须
            // 匹配本次请求里的声明，跨轮结果没有那份声明）。
            ("tool", _) if m.tool_calls.is_none() => {
                let already_nested = wire.iter().any(|n| {
                    n.is_text()
                        && n.tool_call_id == m.tool_call_id
                        && n.node_role() == Some(NodeRole::Tool)
                        && n.content.as_ref().map(|c| c.to_text()) == Some(m.text.clone())
                });
                let declared_here = m
                    .tool_call_id
                    .as_ref()
                    .is_some_and(|id| declared_call_ids.contains(id));
                if !already_nested && !declared_here {
                    wire.push(ChatNode {
                        id: llm_short_id(),
                        role: Some(NodeRole::Assistant),
                        content: Some(MessageContent::Text(format!(
                            "（此前调用工具 {} 的结果）{}",
                            m.tool.as_deref().unwrap_or("<unnamed>"),
                            m.text
                        ))),
                        ..Default::default()
                    });
                }
            }
            _ => {
                wire.push(ChatNode {
                    id: llm_short_id(),
                    role: Some(match m.role.as_str() {
                        "assistant" => NodeRole::Assistant,
                        "tool" => NodeRole::Tool,
                        "system" => NodeRole::System,
                        // 未知角色不默认成 user：那会把「不知道」伪装成「用户说的」。
                        _ => NodeRole::User,
                    }),
                    content: Some(MessageContent::Text(m.text.clone())),
                    tool_call_id: m.tool_call_id.clone(),
                    ..Default::default()
                });
            }
        }
    }
    wire
}

/// 一次往返的产物键名归本插件的行契约；环不解析。
fn finish_str(finish: &ModelFinishReason) -> &str {
    match finish {
        ModelFinishReason::Stop => "stop",
        ModelFinishReason::Length => "length",
        ModelFinishReason::ToolCalls => "tool_calls",
        ModelFinishReason::ContentFilter => "content_filter",
        ModelFinishReason::Other(_) => "other",
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
