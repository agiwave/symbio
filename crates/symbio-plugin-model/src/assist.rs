//! `model.assist` 端口成员：**一次性问答**（D-336）。
//!
//! 与 `model` 端口分开是有判据的：`model` 是对话轮（带工具表、带会话历史、流式帧进
//! 展示面）；`model.assist` 只把一份问答请求发出去、取回正文——不构对话、不带工具、
//! 不留历史。执行体归本插件（端口提供方），提问方的指令随行载荷进来，本插件不认识
//! 「压缩」这类世界语义。
//!
//! ## 载荷
//!
//! [`AssistRequest`]：`system`（提问方指令）+ `user`（正文）。`user` 为空时从事实读面
//! **按角色投影**本轮对话线（用户发言 / 助手终答各一条消息节点，交给 provider 的
//! `flatten_chat_messages` 展平——与主线同一条投影，不在这里拼「用户：…」正文）——
//! 载荷只带身份，正文不进载荷。
//!
//! ## 应答
//!
//! 返回一条载荷 [`AssistReply`] + `root_id` / `final_id`：环把它落成 `response`
//! 事实并补 `port`，读方按 `port == model.assist` 取回。`response` 不进
//! `turn_input` 的 messages（那里只收 `port == model`），所以问答不会污染对话历史。
//!
//! `reasoning` / `reasoning_id` / `cost_ms` 也随回执走：思考段是一次真实往返的
//! 产出，读方（如 `session` 的终答落格）照实复刻，**不在落格侧造理由**。

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::chat_node::{ChatNode, MessageContent};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::port::{
    port_rows, PortErrorClass, PortExecContext, PortExecutor, PortFact, PortInput, PortOutcome,
    PortProvider,
};
use symbio_plugin_api::principal::{Principal, PrincipalKind};
use symbio_plugin_api::{
    llm_short_id, AssistReply, AssistRequest, CapabilityRow, ExecEnv, ExecEventSink, ModelProvider,
    PluginError,
};

/// 一次性问答端口的名字（数据）：名字住 `plugin-api` 的词汇面，提供者与本端口共用
/// 同一常量（D-364）。
pub(crate) use symbio_plugin_api::vocab::PORT_MODEL_ASSIST as ASSIST_PORT;

/// 提供者 id（行名后缀；与 `model` 端口行的提供者区分）。
const ASSIST_PROVIDER: &str = "model-assist";

pub(crate) struct AssistPort {
    active: Arc<dyn ModelProvider>,
    /// 本轮可用的 provider 全集：请求里的偏好按 `provider_id()` 在这里查；
    /// 查不到（或没带偏好）就回落 `active`（= 会话选定）。
    available: Vec<Arc<dyn ModelProvider>>,
}

impl AssistPort {
    pub(crate) fn new(
        active: Arc<dyn ModelProvider>,
        available: Vec<Arc<dyn ModelProvider>>,
    ) -> Self {
        Self { active, available }
    }

    pub(crate) fn rows(self: Arc<Self>) -> Vec<CapabilityRow> {
        port_rows(&(self as Arc<dyn PortProvider>))
    }
}

impl PortProvider for AssistPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ASSIST_PORT]
    }

    fn provider_id(&self) -> &str {
        ASSIST_PROVIDER
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(AssistExecutor {
            active: self.active.clone(),
            available: self.available.clone(),
        }))
    }
}

struct AssistExecutor {
    active: Arc<dyn ModelProvider>,
    available: Vec<Arc<dyn ModelProvider>>,
}

impl AssistExecutor {
    /// 偏好 provider 的解析：**只查这一次请求带进来的 id**。
    ///
    /// 命中 ⇒ 用那个条目；缺席 / 空串 / 查不到 ⇒ 回落会话选定的 `active`。
    /// 「查不到」也算回落而不是失败：一个 typo 不该让问答整条消失（D-348）。
    fn provider_for(&self, preferred: Option<&str>) -> Arc<dyn ModelProvider> {
        preferred
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|id| {
                self.available
                    .iter()
                    .find(|provider| provider.provider_id() == id)
                    .cloned()
            })
            .unwrap_or_else(|| self.active.clone())
    }
}

#[async_trait::async_trait]
impl PortExecutor for AssistExecutor {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        // 共享载荷是唯一的输入形状；无法反序列化 ⇒ 这一笔响亮失败。
        let request: AssistRequest = match serde_json::from_value(input.payload.clone()) {
            Ok(request) => request,
            Err(error) => {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("问答载荷不可读：{error}"),
                )
            }
        };
        let root_id = input
            .payload
            .get("root_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let final_id = input
            .payload
            .get("final_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        // 切片计划（可选）：提问方（如 `compact`）只压「装得下的最旧一段」，
        // 用两端事实 id + 每条的头尾字符预算表达。它只约束这一段怎么装，
        // 不改变角色投影——正文仍从事实读，本执行体不认识「压缩」。
        let slice = match TranscriptSlice::from_payload(&input.payload) {
            Ok(slice) => slice,
            Err(error) => return PortOutcome::failed(PortErrorClass::NonRetryable, error),
        };

        // 对话线投影：一条事实一条节点，角色由事实给；正文原样交出，由 provider 侧的
        // `flatten_chat_messages` 折成协议消息。这里**不再拼**「用户：…助手：…」——
        // 那会造出第二个投影器，请求形状与主线漂移（t61 / LC-007）。
        let wire = if request.user.trim().is_empty() {
            let facts = exec
                .reader
                .as_ref()
                .map(|reader| reader.facts())
                .unwrap_or_default();
            match transcript_nodes(&facts, slice.as_ref()) {
                Ok(nodes) => nodes,
                // 切片端点在读面找不到 ⇒ 这一笔**响亮失败**，绝不退回整份转写：
                // 那正是「请求装不下」的原始故障（D-401）。
                Err(error) => return PortOutcome::failed(PortErrorClass::NonRetryable, error),
            }
        } else {
            // 身份写 `principal`（D-147 / S-014）：`role` 只是存量反序列化落点。
            vec![ChatNode {
                id: llm_short_id(),
                principal: Some(Principal::new(PrincipalKind::Person, "person/local")),
                content: Some(MessageContent::Text(request.user.clone())),
                ..Default::default()
            }]
        };
        // 一条消息、无工具表、静默出口：问答不参与对话轮，也不往展示面写帧。
        if wire.is_empty() {
            return PortOutcome::ok();
        }
        let env = ExecEnv::new(ExecEventSink::silent(), exec.abort.clone());
        let started = std::time::Instant::now();
        // 能力方带进来的偏好优先；没带 / 取不到 ⇒ 会话选定（这一个端口实例的 active）。
        let provider = self.provider_for(request.provider.as_deref());
        match provider
            // internal-model-allow IM-001: 问答端口是静默的一次性往返（无工具、无会话历史、不参与档位闸门）；走 `for_turn` 会把问答伪装成对话轮。
            .round_trip(&request.system, &wire, &[], &root_id, &env)
            .await
        {
            Ok(output) => {
                let text = output.text.trim().to_string();
                // 思考段与正文同源（有正文才可能有思考）：如实带回，读方不得自造。
                let reasoning = (!output.reasoning.trim().is_empty())
                    .then(|| output.reasoning.trim().to_string());
                let reasoning_id = reasoning.as_ref().map(|_| llm_short_id());
                let reply = AssistReply {
                    text,
                    reasoning,
                    reasoning_id,
                    // cost_ms 与对话轮同一条实测口径（执行体是 adapter 边界）：
                    // 内部往返本身不写事件，但这一次的耗时随回执交给落格方。
                    cost_ms: started.elapsed().as_millis() as u64,
                };
                PortOutcome {
                    facts: vec![json!({
                        "root_id": root_id,
                        "final_id": final_id,
                        "text": reply.text,
                        "reasoning": reply.reasoning,
                        "reasoning_id": reply.reasoning_id,
                        "cost_ms": reply.cost_ms,
                    })],
                    ..PortOutcome::ok()
                }
            }
            Err(PluginError::RateLimited(why)) => {
                PortOutcome::failed(PortErrorClass::Retryable, why)
            }
            Err(error) => PortOutcome::failed(PortErrorClass::NonRetryable, error.to_string()),
        }
    }
}

/// 摘要输入可选的**切片计划**：只取两条端点事实之间（含两端）的对话线，
/// 并按 `truncate` 对某条正文做头尾截断。
///
/// 形状由提问方在载荷里给（`slice: { from, to, truncate: { <id>: { head_chars,
/// tail_chars } } }`）；缺 `slice` = 整份转写（通用问答的既有形状）。两端端点或
/// 截断参数读不出 ⇒ 这一笔失败——执行体**不得**用「找不到就退回整份」来掩盖。
struct TranscriptSlice {
    from: String,
    to: String,
    truncate: BTreeMap<String, (usize, usize)>,
}

impl TranscriptSlice {
    fn from_payload(payload: &Value) -> Result<Option<Self>, String> {
        let Some(slice) = payload.get("slice") else {
            return Ok(None);
        };
        let text = |key: &str| {
            slice
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("摘要切片缺 `{key}`"))
        };
        let from = text("from")?;
        let to = text("to")?;
        let mut truncate = BTreeMap::new();
        if let Some(map) = slice.get("truncate").and_then(Value::as_object) {
            for (id, spec) in map {
                let head = spec.get("head_chars").and_then(Value::as_u64);
                let tail = spec.get("tail_chars").and_then(Value::as_u64);
                let (Some(head), Some(tail)) = (head, tail) else {
                    return Err(format!("摘要切片 `{id}` 的截断参数不可读"));
                };
                truncate.insert(id.clone(), (head as usize, tail as usize));
            }
        }
        Ok(Some(Self { from, to, truncate }))
    }

    /// 按切片取正文；`None` = 整份。
    fn window<'a>(&self, facts: &'a [PortFact]) -> Result<&'a [PortFact], String> {
        let from = facts
            .iter()
            .position(|fact| fact.id == self.from)
            .ok_or_else(|| format!("摘要切片起点 `{}` 不在事实读面", self.from))?;
        let to = facts
            .iter()
            .position(|fact| fact.id == self.to)
            .ok_or_else(|| format!("摘要切片终点 `{}` 不在事实读面", self.to))?;
        if from > to {
            return Err(format!(
                "摘要切片端点顺序颠倒（{} 在 {} 之后）",
                self.from, self.to
            ));
        }
        Ok(&facts[from..=to])
    }

    /// 一条正文按计划取头尾（按字符截：本仓估算器里 1 字符 ≤ 1 token，是硬上限）。
    fn body(&self, fact: &PortFact, text: &str) -> String {
        let Some((head_chars, tail_chars)) = self.truncate.get(&fact.id) else {
            return text.to_string();
        };
        let chars: Vec<char> = text.chars().collect();
        let head: String = chars.iter().take(*head_chars).collect();
        let tail: String = chars
            .iter()
            .skip(chars.len().saturating_sub(*tail_chars))
            .collect();
        format!("{head}…中间省略…\n{tail}")
    }
}

/// 本轮对话线：用户发言 / 助手终答各自一条**带主体**的消息节点，按事实顺序。
///
/// 主体族决定读侧角色（`Person → user`、`Agent → assistant`）：role 是请求形状的
/// 一部分（对话面判据 t61 数的是 `role == user` 的条数），所以由事实种类决定，
/// 不靠正文前缀表达，也不直写存量 `role` 格（S-014）。
fn transcript_nodes(
    facts: &[PortFact],
    slice: Option<&TranscriptSlice>,
) -> Result<Vec<ChatNode>, String> {
    let window = match slice {
        Some(slice) => slice.window(facts)?,
        None => facts,
    };
    let mut nodes = Vec::new();
    for fact in window {
        let (principal, text) = match fact.kind.as_str() {
            EVENT_USER_MESSAGE => (
                Principal::new(PrincipalKind::Person, "person/local"),
                fact.payload.get("text"),
            ),
            EVENT_CHAT_ASSISTANT_FINAL => (
                // plugin-entry-allow E-005: `session/main` 是会话主体 id（Principal），不是插件路由地址
                Principal::new(PrincipalKind::Agent, "session/main"),
                fact.payload
                    .get("text")
                    .or_else(|| fact.payload.get("answer")),
            ),
            _ => continue,
        };
        let Some(text) = text.and_then(serde_json::Value::as_str) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let text = match slice {
            Some(slice) => slice.body(fact, text.trim()),
            None => text.trim().to_string(),
        };
        nodes.push(ChatNode {
            id: llm_short_id(),
            principal: Some(principal),
            content: Some(MessageContent::Text(text)),
            ..Default::default()
        });
    }
    Ok(nodes)
}

#[cfg(test)]
#[path = "assist.test.rs"]
mod tests;
