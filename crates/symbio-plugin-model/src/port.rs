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
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::ports;
use symbio_plugin_api::{
    llm_short_id, CapabilityMeta, CapabilityRow, ExecAbortSignal, ExecEnv, ExecEventSink,
    ExecTranscriptWriter, LlmAdapter, ModelFinishReason, ModelProvider, PluginError, PromptMessage,
    ProviderLlmAdapter,
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

    /// 通用执行面（`DE-047` D-305 裁决一）：一次往返的请求构造、流式帧、工具调用
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
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("模型请求载荷不可读：{error}"),
                )
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
        match provider
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
            Ok(output) => {
                let calls: Vec<serde_json::Value> = output
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "wire_id": call.wire_id,
                            "name": call.name,
                            "arguments": call.arguments,
                            "parse_error": call.parse_error,
                            "overflow_notice": call.overflow_notice,
                        })
                    })
                    .collect();
                // 一次往返的产物 → 事实草稿（键名是本插件的行契约，环不解析）。
                PortOutcome {
                    facts: vec![serde_json::json!({
                        "text": output.text,
                        "reasoning": output.reasoning,
                        "response_id": output.response_id,
                        "tool_calls": calls,
                        "finish": finish_str(&output.finish),
                        // 终止判定归执行体（D-305 裁决一）：有工具调用 ⇒ 还要再派一批；
                        // 没有 ⇒ 这一轮答完了。环只照抄，不再按世界分支。
                        "decision": if output.tool_calls.is_empty() { "halt" } else { "tools" },
                        "cost_ms": started.elapsed().as_millis() as u64,
                        "usage": output.usage.as_ref().map(|u| serde_json::json!({
                            "input": u.input,
                            "output": u.output,
                        })),
                        "spend": output.spend.as_ref().map(|s| serde_json::json!({
                            "amount_nano": s.amount_nano,
                            "currency": s.currency,
                        })),
                        "response_text_child_id": output.response_text_child_id,
                        "reasoning_child_id": output.reasoning_child_id,
                        // 执行体自己持有的流式读数：节点 id 与半截正文。成功时可与
                        // `output.*` 对上；中止时它就是唯一来源（环不再从桥取）。
                        "stream": rows.snapshot(),
                    })],
                    error: None,
                }
            }
            Err(PluginError::Aborted) => PortOutcome {
                // 半截产出仍要留痕：中止是这一笔的状态，不是「没有发生过」。
                facts: vec![serde_json::json!({
                    "status": "aborted",
                    "stream": rows.snapshot(),
                })],
                error: Some((PortErrorClass::NonRetryable, "aborted".to_string())),
            },
            Err(PluginError::RateLimited(why)) => {
                with_stream(PortOutcome::failed(PortErrorClass::Retryable, why), &rows)
            }
            Err(error) => with_stream(
                PortOutcome::failed(PortErrorClass::NonRetryable, error.to_string()),
                &rows,
            ),
        }
    }
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
            "response_text_child_id": state.text_id,
            "reasoning_child_id": state.reasoning_id,
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

/// 失败载荷也带上流式读数：中止 / 限流之后那段半截正文只活在这里。
fn with_stream(mut outcome: PortOutcome, rows: &StreamRows) -> PortOutcome {
    let stream = rows.snapshot();
    match outcome.facts.first_mut() {
        Some(first) => {
            if let Some(object) = first.as_object_mut() {
                object.insert("stream".to_string(), stream);
            }
        }
        None => outcome
            .facts
            .push(serde_json::json!({ "stream": stream })),
    }
    outcome
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
