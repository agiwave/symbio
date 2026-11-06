//! model 端口的世界提供者（D-298 M1）：解析与适配器构造收在 model 插件一侧。
//!
//! 环（loop）只按端口名取——provider 怎么解析（严格查 `preferred` → 落回当次
//! 生效者）、适配器怎么构造，都是这里的私事。`ModelPort` 在 traverse 里随能力
//! 目录一起装：生效者缺席 ⇒ 不注册端口行 ⇒ 环在调用点响亮失败（落失败事实
//! 并中止），而不是在这里猜一个别的模型。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use symbio_plugin_api::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, CHAT_STYLE_TAG_RUN, CHAT_STYLE_TAG_TOOL,
};
use symbio_plugin_api::context::{
    sections, Cadence, ContextRequest, ContextSection, DelegateVerdict, LiveInputs,
};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_AUTHZ_DENIED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_MESSAGE_ADDED,
    EVENT_SESSION_CONFIG, EVENT_TOOL_REQUESTED, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFact, PortFactEmit, PortInput, PortOutcome,
    PortProvider, PortReply,
};
use symbio_plugin_api::{
    llm_short_id, CapabilityMeta, CapabilityRow, Entity, ExecEnv, ExecEventSink,
    ExecTranscriptWriter, FactDraft, ModelFinishReason, ModelProvider, PluginError,
    PluginInvokeRequestExt, PromptMessage, TurnOutput, TurnToolCallInfo, Verb, EVENT_FAILED,
    EVENT_RESPONSE,
};

/// 本插件服务的端口名（提供者自己定义；契约层不认识它）。
pub(crate) const MODEL_PORT: &str = "model";

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
        &[MODEL_PORT]
    }

    fn provider_id(&self) -> &str {
        self.active.provider_id()
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
///
/// 这里只有**身份与选择**：正文输入（system prompt / messages / tools）由执行体
/// 从事实读面取——载荷不装载它们（`D` 陈述的执行体自解释）。
#[derive(Debug, serde::Deserialize)]
struct RoundTripRequest {
    /// 首选 provider id；不在目录 ⇒ 落回当次生效者（与 `llm_round_trip` 同一条规则）。
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    root_id: String,
    /// 记账与装配用的容器事实（`D-314`）：`response` 载荷的 `root_id` / `principal`、
    /// `calls` 的 `call_key` 都从这里取，环不再代装配。
    #[serde(default)]
    principal: String,
}

/// 一次往返的输入视图：**全从事实读**（写方是谁不问）。
///
/// - system prompt / tools：最近一条 `session.config` 事实的同名字段（无则空）；
/// - messages：`user.message`、本端口已落的 `response`、工具产物 `artifact.added`、
///   以及轮内追加的 `message.added`（含 `elision` 的省略形态，D-249）。
///
/// 返回值第四格是本轮**第几次**模型请求（0 = 首幕）：`message.added` 与省略区间
/// 都需要它之外的历史，而发言级上下文段只在首幕取一次（D-220 / D-252）。
fn turn_input(
    facts: &[PortFact],
    events: &[symbio_plugin_api::Event],
) -> (String, Vec<CapabilityMeta>, Vec<PromptMessage>, u32) {
    let config = facts
        .iter()
        .rev()
        .find(|fact| fact.kind == EVENT_SESSION_CONFIG);
    let system_prompt = config
        .and_then(|fact| fact.payload.get("system_prompt"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tools = config
        .and_then(|fact| fact.payload.get("tools"))
        .and_then(|value| serde_json::from_value::<Vec<CapabilityMeta>>(value.clone()).ok())
        .unwrap_or_default();

    // `message.added` 的省略形态：`elision` 记的是**被取代的入库区间**（seq 闭区间）。
    // 区间是事实的身份，不在载荷里猜：`facts` 的 id 与 `events` 的 id 同源。
    let seq_of: HashMap<&str, u64> = events
        .iter()
        .filter_map(|e| e.seq.as_ref().map(|s| (e.event_id.as_str(), s.value())))
        .collect();
    let elided: Vec<(u64, u64)> = facts
        .iter()
        .filter(|fact| fact.kind == EVENT_MESSAGE_ADDED)
        .filter_map(|fact| {
            let elision = fact.payload.get("elision")?;
            Some((elision.get("from")?.as_u64()?, elision.get("to")?.as_u64()?))
        })
        .collect();

    let mut messages = Vec::new();
    // 被省略区间的工具结果：连它的**调用**一起不要。留一条没有结果的调用，
    // 降级路径会补一条「执行被中断」的占位 tool 消息——省略就漏了（t91）。
    let elided_calls: Vec<&str> = facts
        .iter()
        .filter(|fact| fact.kind == EVENT_ARTIFACT_ADDED)
        .filter(|fact| {
            seq_of
                .get(fact.id.as_str())
                .is_some_and(|seq| elided.iter().any(|(from, to)| (*from..=*to).contains(seq)))
        })
        .filter_map(|fact| {
            fact.payload
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    // 本轮第几次模型请求（0 = 首幕）：最后一条 `user.message` 之后数应答轮。
    let mut round = 0u32;
    // 同一句话可能落两格：模型行的 `response`（环的收据）与会话的
    // `chat.assistant.final`（轮末投影）在收束轮里带**同一个 `text_id`**——那是同一个
    // 文本节点的身份。回灌只认第一格，否则同一句话在请求里出现两次（t46 基线第 7 条）。
    let mut seen_texts: HashSet<String> = HashSet::new();
    for fact in facts {
        if let Some(seq) = seq_of.get(fact.id.as_str()) {
            if elided.iter().any(|(from, to)| (*from..=*to).contains(seq)) {
                continue;
            }
        }
        match fact.kind.as_str() {
            EVENT_USER_MESSAGE => {
                round = 0;
                if let Some(text) = fact.payload.get("text").and_then(serde_json::Value::as_str) {
                    if !text.trim().is_empty() {
                        messages.push(PromptMessage {
                            role: "user".into(),
                            text: text.to_string(),
                            tool_call_id: None,
                            tool: None,
                            tool_calls: None,
                        });
                    }
                }
            }
            // 轮内追加的消息（D-249 的压缩节点 / 省略说明）：id 写的是它取代的区间，
            // 这里按节点自己的身份回灌成一条普通消息——读侧不解释「为什么」。
            EVENT_MESSAGE_ADDED => {
                let nodes = fact
                    .payload
                    .get("messages")
                    .and_then(serde_json::Value::as_array)
                    .map(|nodes| {
                        nodes
                            .iter()
                            .filter_map(|node| {
                                serde_json::from_value::<ChatNode>(node.clone()).ok()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for node in nodes {
                    let role = match node.node_role().unwrap_or(NodeRole::User) {
                        NodeRole::User => "user",
                        NodeRole::Assistant => "assistant",
                        NodeRole::Tool => "tool",
                        NodeRole::System => "system",
                    };
                    let text = node
                        .content
                        .as_ref()
                        .map(MessageContent::to_text)
                        .unwrap_or_default();
                    if !text.trim().is_empty() {
                        messages.push(PromptMessage {
                            role: role.into(),
                            text,
                            tool_call_id: None,
                            tool: None,
                            tool_calls: None,
                        });
                    }
                }
            }
            EVENT_RESPONSE if fact.port == MODEL_PORT => {
                // 收束轮可能与 `chat.assistant.final` 同文本（见 `first_text`）。
                if !first_text(&mut seen_texts, fact) {
                    continue;
                }
                round = round.saturating_add(1);
                let text = fact
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                // 声明里的调用身份**只取 provider 交回的 wire id**：`tool.requested` /
                // `artifact.added` / 工具结果用的都是它。载荷里的 `id`（节点身份）只作
                // 显示与链接配对，不参与任何匹配——两套 id 混用会让装配误判「结果没
                // 嵌进来」，于是多推一条降级的 assistant 复读（t3 的断点）。
                let tool_calls = fact
                    .payload
                    .get("tool_calls")
                    .and_then(serde_json::Value::as_array)
                    .map(|calls| {
                        let root = fact
                            .payload
                            .get("root_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        calls
                            .iter()
                            .enumerate()
                            .map(|(idx, c)| {
                                let name = c
                                    .get("name")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("<unnamed>");
                                // 与 `dispatch_calls` 同一条兜底：provider 没给 id 时两边
                                // 必须写出同一个合成键，否则工具结果的 `tool_call_id`
                                // 与声明对不上，复读又回来。
                                let id = c
                                    .get("wire_id")
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|s| !s.trim().is_empty())
                                    .or_else(|| c.get("id").and_then(serde_json::Value::as_str))
                                    .map(str::to_string)
                                    .unwrap_or_else(|| format!("call_{}_{}_{}", name, root, idx));
                                symbio_plugin_api::PromptToolCall {
                                    id,
                                    name: name.to_string(),
                                    arguments: c
                                        .get("arguments")
                                        .cloned()
                                        .unwrap_or(serde_json::Value::Null),
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .map(|calls| {
                        calls
                            .into_iter()
                            .filter(|call| !elided_calls.contains(&call.id.as_str()))
                            .collect::<Vec<_>>()
                    })
                    .filter(|calls: &Vec<symbio_plugin_api::PromptToolCall>| !calls.is_empty());
                if !text.trim().is_empty() || tool_calls.as_ref().is_some_and(|c| !c.is_empty()) {
                    messages.push(PromptMessage {
                        role: "assistant".into(),
                        text,
                        tool_call_id: None,
                        tool: None,
                        tool_calls,
                    });
                }
            }
            // 终答也要回灌正文：收束轮可能只有 `chat.assistant.final`（没有模型行的
            // `response`）——漏了它下一轮的对话线就断一截。两格同时存在时由
            // [`first_text`] 按 `text_id` 去重，同一句只进请求一次。
            EVENT_CHAT_ASSISTANT_FINAL => {
                if !first_text(&mut seen_texts, fact) {
                    continue;
                }
                round = round.saturating_add(1);
                if let Some(text) = fact.payload.get("text").and_then(serde_json::Value::as_str) {
                    if !text.trim().is_empty() {
                        messages.push(PromptMessage {
                            role: "assistant".into(),
                            text: text.to_string(),
                            tool_call_id: None,
                            tool: None,
                            tool_calls: None,
                        });
                    }
                }
            }
            EVENT_ARTIFACT_ADDED => {
                let (Some(tool), Some(call_id)) = (
                    fact.payload.get("tool").and_then(serde_json::Value::as_str),
                    fact.payload
                        .get("tool_call_id")
                        .and_then(serde_json::Value::as_str),
                ) else {
                    continue;
                };
                let text = fact
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                messages.push(PromptMessage {
                    role: "tool".into(),
                    text: text.to_string(),
                    tool_call_id: Some(call_id.to_string()),
                    tool: Some(tool.to_string()),
                    tool_calls: None,
                });
            }
            _ => {}
        }
    }
    (system_prompt, tools, messages, round)
}

/// 这条应答事实的文本节点身份是否**首次出现**。
///
/// 收束轮把同一句终答落两格：模型行的 `response`（环的收据）与会话的
/// `chat.assistant.final`（轮末投影），两者的 `text_id` 不变。回灌只认第一格，
/// 否则请求里同一句话出现两次。`text_id` 缺失/为空 ⇒ 没有身份可比，返回 `true`
/// （宁可重复，也不误删）。
fn first_text(seen: &mut HashSet<String>, fact: &PortFact) -> bool {
    let Some(id) = fact
        .payload
        .get("text_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return true;
    };
    seen.insert(id.to_string())
}

/// 装配侧取段（D-208 / D-220）：发言级段只在首幕取一次，请求级段每次请求都取。
///
/// 段身份来自 provider 自己声明的 [`ContextSection::slot`]（宿主不拼名字）；
/// 这里只做「空段过滤 + 按节奏追加」，判断仍留在 provider 侧。
fn context_sections(
    events: &[symbio_plugin_api::Event],
    tools: &[CapabilityMeta],
    principal: &str,
    round: u32,
) -> Vec<ContextSection> {
    let query = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .and_then(|e| e.payload.get("text").and_then(serde_json::Value::as_str));
    let base = ContextRequest {
        request_round: round,
        principal,
        query,
        facts: events,
        recall: None,
        live: None,
    };
    let mut out = Vec::new();
    if round == 0 {
        out.extend(sections(&base, Cadence::Utterance));
    }
    let live = LiveInputs {
        tools,
        // Q1 的判定归控制面；模型端口这里只负责「请求已经发出」，不代判委派。
        delegate: DelegateVerdict::NotJudged,
        workers: &[],
    };
    let with_live = ContextRequest {
        live: Some(&live),
        ..base
    };
    out.extend(sections(&with_live, Cadence::Request));
    out.retain(|section| !section.text.trim().is_empty());
    out
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
                    "failed-outcome".to_string(),
                    EVENT_FAILED,
                    Verb::Asserted,
                    serde_json::json!({ "status": "failed", "why": &why }),
                )
                .await;
                return PortOutcome::failed(PortErrorClass::NonRetryable, why);
            }
        };
        // 事实读面由宿主实现、环只转交：正文输入全从事实取，载荷只带身份与选择。
        let events = exec
            .reader
            .as_ref()
            .map(|reader| reader.events())
            .unwrap_or_default();
        let facts = exec
            .reader
            .as_ref()
            .map(|reader| reader.facts())
            .unwrap_or_default();
        let (system_prompt, tools, messages, round) = turn_input(&facts, &events);
        let provider = request
            .provider
            .as_deref()
            .and_then(|id| self.catalog.iter().find(|p| p.provider_id() == id))
            .unwrap_or(&self.active);
        // 请求构造归执行体（D-305 裁决一 / D-306 裁决二）：环交进的是**值**
        // （`PromptMessage`），线格式在这里收口。
        // 段注入（D-220 / D-252）：发言级段与请求级段都拼在**历史之前**、`role: system`。
        let context = context_sections(&events, &tools, &request.principal, round);
        let mut prompt = Vec::with_capacity(context.len() + messages.len());
        for section in context {
            prompt.push(PromptMessage {
                role: "system".into(),
                text: section.text,
                tool_call_id: None,
                tool: None,
                tool_calls: None,
            });
        }
        prompt.extend(messages);
        let wire = wire_messages(&prompt);
        let started = std::time::Instant::now();
        // 流式增量是**瞬时帧**，不进事实表（`D-314` 两条通道）：provider 往 `env.sink()`
        // 写帧，这一出口记下节点 id 与半截正文（供 `response` / `failed` 载荷使用），
        // 再原样转发给展示出口。丢帧不影响正确性——终稿以 `response` 为准。
        let rows = Arc::new(StreamRows::new(exec.sink.clone()));
        let env = ExecEnv::new(ExecEventSink::direct(rows.clone()), exec.abort.clone());
        let mut world_kind = EVENT_FAILED;
        // 工具轮：**一次一轮一条** `tool.requested`（载荷 = 本轮调用清单）。
        let mut requested: Option<serde_json::Value> = None;
        let outcome = match provider
            // internal-model-allow IM-001/gate: 端口执行体转发欠债行给的工具集（D-305 裁决一）。
            .round_trip(&system_prompt, &wire, &tools, &request.root_id, &env)
            .await
        {
            Ok(output) => {
                // 终答是**独立事实种类**（`chat.assistant.final`）：只有收束轮（无工具调用）
                // 才落它。债表据此排除「这一轮已答完」，而环的 `request` / `sent` / `retry`
                // 因种类不同不误命中——可重试失败后的重试轮不会被提前跳过。
                let is_final = output.tool_calls.is_empty();
                if is_final {
                    world_kind = EVENT_CHAT_ASSISTANT_FINAL;
                } else {
                    world_kind = EVENT_RESPONSE;
                }
                // 应答承载（D-319）：assistant 正文 + 逐条 `tool_calls` 声明在这里组装，
                // 环只把 `messages` 按序追加、按 `links` 配对。id 取法与环侧旧实现同源
                // （wire id → 节点 id → 合成 `call_<名>_<轮>_<序号>`）。
                let mut messages: Vec<PromptMessage> = Vec::new();
                let mut links = Vec::new();
                // 派发清单只有**一份**构造：`response.calls` 与 `tool.requested.calls`
                // 读的是同一个向量。此前这里另写一遍（只挑 id/name/arguments），
                // 丢弃标记就是在那一遍里被剥掉的——同一笔调用于是带着「未执行」的
                // 正文被执行、收据还记成成功。
                let calls = dispatch_calls(&request, &output.tool_calls);
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
                    let id = calls[index]["id"].as_str().unwrap_or_default().to_string();
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
                if !calls.is_empty() {
                    requested = Some(serde_json::json!({
                        "root_id": request.root_id,
                        "calls": calls,
                    }));
                }
                // 一次往返的终稿 → `response` 载荷：正文 / 思考、节点身份、终答折叠、
                // 用量与工具调用集合展开都由执行体装配（`D-314`）。
                let mut payload =
                    outcome_payload(&request, Some(&output), &rows.snapshot(), started, None);
                // 上下文水位：收束轮才写（中途工具轮的请求体积不代表最终水位）。
                // 实测输入用量缺席 ⇒ 整格不写——「没测到」不是「用量 0」。
                if is_final {
                    if let Some(watermark) =
                        context_watermark(&output, provider.effective_context_tokens().await)
                    {
                        payload["context"] = watermark;
                    }
                }
                PortOutcome {
                    facts: vec![payload],
                    error: None,
                    reply: PortReply { messages, links },
                }
            }
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
        // 收束闸 · 发言臂（D-067）：主体未持 `reply.*` ⇒ 不落 `chat.assistant.final`。
        // `session.close-turn` 以它为欠债来源（`Decl.owes.source`），不落它 ⇒ 本轮
        // 派生事实（记忆 / 任务 / 承诺）与 `session.settled` 都不产生——拒绝要落到
        // 收束层（t73）。答案仍经 `reply` 回给调用方：收束格是记录，不是呈现。
        //
        // 判的对象取**请求上下文的主体**（`ctx[AGENT_ID]` 的纯派生），与工具端
        // 同一取值口径；载荷里的 `principal` 是记账字段，不是授权判据。
        let Some(agent_id) = exec
            .ctx
            .as_deref()
            .and_then(|ctx| ctx.get(symbio_plugin_api::AGENT_ID))
        else {
            // 主体缺失就是装配错误：**不回落** `session/main`，也不回落 `"user"`。
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "模型执行体缺少会话主体：装配层未设 AGENT_ID（拒绝回落）",
            );
        };
        let gate_principal = symbio_plugin_api::principal_of(Some(agent_id.as_str()));
        let speech_denied = outcome.error.is_none()
            && world_kind == EVENT_CHAT_ASSISTANT_FINAL
            && !symbio_plugin_api::production_matrix().can_reply(&gate_principal);
        // 账：`chat.assistant.final` / `response` / `failed` 由执行体经宿主写口落格（`D-314`）；
        // 信封（`verb`）由这里按事实种类给，键由 `emit_outcome` 按同一身份构造。
        if let Some(payload) = outcome.facts.first() {
            let kind = if outcome.error.is_none() {
                world_kind
            } else {
                EVENT_FAILED
            };
            if speech_denied {
                // 拒绝也要留痕：重建面按 `key = chat.assistant.final` + `root_id` 配对，
                // 把这一轮的答复挡在对话面之外（`rebuild::denied_speech`）。
                emit_outcome(
                    &exec.emit,
                    format!("deny-{}-speech", request.root_id),
                    EVENT_AUTHZ_DENIED,
                    Verb::Asserted,
                    serde_json::json!({
                        "port": MODEL_PORT,
                        "key": EVENT_CHAT_ASSISTANT_FINAL,
                        "root_id": &request.root_id,
                        "detail": &request.principal,
                    }),
                )
                .await;
            } else {
                let (key, verb) = if kind == EVENT_CHAT_ASSISTANT_FINAL {
                    (format!("{kind}:{}", request.root_id), Verb::Closed)
                } else {
                    (format!("{kind}-outcome"), Verb::Asserted)
                };
                emit_outcome(&exec.emit, key, kind, verb, payload.clone()).await;
            }
        }
        // 工具轮的世界事实：债表只问「有没有 `tool.requested`」，执行体自己从读面取清单。
        // 身份取本次派发的幂等键（一轮一次，跨轮天然不同），不靠计数。
        if let Some(payload) = requested {
            emit_outcome(
                &exec.emit,
                format!("{EVENT_TOOL_REQUESTED}:{}", exec.idempotency_key),
                EVENT_TOOL_REQUESTED,
                Verb::Asserted,
                payload,
            )
            .await;
        }
        outcome
    }
}

/// 执行体落协议事实的统一出口（`D-314`）：键与信封由这里构造，只交载荷。
async fn emit_outcome(
    emit: &PortFactEmit,
    key: String,
    kind: &str,
    verb: Verb,
    payload: serde_json::Value,
) {
    emit.emit(vec![FactDraft {
        key,
        kind: kind.to_string(),
        entity: Entity::Scope,
        verb,
        payload,
        produced_by: None,
    }])
    .await;
}

/// 收束轮的上下文水位（终答载荷的 `context` 格，读侧折进 Turn 骨架 `meta.context`）。
///
/// 只写**测得到**的数：`request_tokens` 取 provider 回报的实测输入用量，缺席 ⇒ 整格
/// 缺席（不补 0）；`limit_tokens` 取本 provider 的有效上下文上限。
///
/// `compress_tokens` 需要 provider 的 `reserved_tokens`，而 `ModelProvider` 不暴露它——
/// 按「不为读数扩 trait」留缺：读数退化成「已用 / 上限」，不自己发明阈值。
fn context_watermark(output: &TurnOutput, limit: u32) -> Option<serde_json::Value> {
    let request_tokens = output.usage.as_ref().and_then(|usage| usage.input)?;
    Some(serde_json::json!({
        "request_tokens": request_tokens,
        "limit_tokens": limit,
    }))
}

/// 一次往返写进 `response` / `failed` 的载荷（行契约归本插件，`D-314`）。
///
/// 键名与环侧 `output_fields` 同义：`text` / `text_id` / `reasoning` / `reasoning_id` /
/// `answer` / `answer_id` / `truncated` / `principal` / `root_id`，外加本轮的工具调用
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
        .or_else(|| (!text.trim().is_empty()).then(|| format!("{}-text", request.root_id)));
    let reasoning_id = stream_id("reasoning_id")
        .or_else(|| {
            output
                .map(|o| o.reasoning_child_id.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            (!reasoning.trim().is_empty()).then(|| format!("{}-reasoning", request.root_id))
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
        "root_id": request.root_id,
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
        // `id` 是节点身份（流式面上那张调用卡），**不参与任何匹配**；匹配只认
        // `wire_id`（provider 交回的 tool_call_id，见 `turn_input` 的声明装配）。
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
/// 一次调用只有一个身份：**provider 交回的 `tool_call_id`（wire）**。派发、工具债、
/// `artifact.added` 的 `tool_call_id`、下一轮提示词装配全都用它。节点 id 只在
/// provider 没给 id 时兜底，绝不参与匹配（t3：两套 id 混用导致重复复读与二次执行）。
fn dispatch_calls(
    request: &RoundTripRequest,
    tool_calls: &[TurnToolCallInfo],
) -> Vec<serde_json::Value> {
    tool_calls
        .iter()
        .enumerate()
        .map(|(idx, call)| {
            let id = call
                .wire_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| call.id.clone().filter(|s| !s.trim().is_empty()))
                // 与提示词装配同一条兜底（`call_<名>_<root>_<序号>`）：两处写出的
                // 调用键必须相等，否则下一轮又把「同一笔调用」认成两笔。
                .unwrap_or_else(|| {
                    let name = call.name.as_deref().unwrap_or("<unnamed>");
                    format!("call_{}_{}_{}", name, request.root_id, idx)
                });
            let mut entry = serde_json::json!({
                "id": id,
                "name": call.name.clone().unwrap_or_default(),
                "arguments": call.arguments.clone(),
            });
            // 工具卡在实时面上的节点身份（`tool_calls[].id`），与 wire id 是两套：
            // 执行方据此把「开始执行」的状态帧写到**那张卡**上（`meta.started_at`）。
            if let Some(node_id) = call.id.clone().filter(|s| !s.trim().is_empty()) {
                entry["node_id"] = serde_json::json!(node_id);
            }
            // 执行层判「不执行」的成因随清单走：`overflow_notice` 是模型侧在流收口时
            // 写下的丢弃标记（正文即反馈）。它若被派发清单剥掉，执行层就只看得到
            // 「一条普通调用」，只好照常执行并把产物记成成功——同一笔调用因此出现
            // 「结果说未执行、收据说成功」两份互相打脸的说法。
            if let Some(notice) = call.overflow_notice.as_ref() {
                entry["overflow_notice"] = serde_json::json!(notice);
            }
            entry
        })
        .collect()
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
