//! ProviderLlmAdapter —— ⑤ `LlmAdapter` 端口的**真实实现**（生产接线点）。
//!
//! S2 彩排用的确定性桩 `StubLlmAdapter`（`#[cfg(test)]`）走通闭环形状；本适配器把端口接到
//! **真实传输层**：持 `Arc<dyn ModelProvider>`（core 契约），`generate` 经
//! `execute_turn` 走「请求体构造 → 真实 HTTP POST → SSE 流解析 → 文本聚合」。
//! 会话链路（chat_loop / classify / compose）与 core 彩排链路（`Reasoner`）
//! 从此共享同一个真实模型入口——**换适配器不换链路**。
//!
//! 依赖方向：本文件只依赖 core 契约（`ModelProvider` / `LlmAdapter` /
//! `ExecEventSink` 均在 core）——纯胶水就该住在契约旁边。真实传输层的全链路
//! 彩排（真实 TCP + HTTP + SSE）挂在 **`plugins/model/bound_provider.test.rs`**：
//! 那份测试要装配 `BoundProvider` / `openai_chat` 等插件内部件，而
//! `plugins::model` 是私有模块（core 侧**在可见性上**就到不了它）——
//! 依赖方向由编译器保证，不靠约定。

use std::sync::Arc;

use async_trait::async_trait;

use super::{AdapterError, DeltaSink, FullModel, LlmAdapter, LlmTurn, SilentDeltas};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::CapabilityMeta;
use crate::symbio_core::ExecAbortSignal;
use crate::symbio_core::ExecEnv;
use crate::symbio_core::ExecEventSink;
use crate::symbio_core::PromptMessage;
use crate::symbio_core::{llm_short_id, ExecTranscriptWriter, ModelProvider, PluginError};

/// 流式帧桥：把 `execute_turn` 的转写帧**择要**转成增量——正文与推理**两条流**
/// 各自转发（[`DeltaSink::on_delta`] / [`DeltaSink::on_reasoning`]）：
/// - 快照帧（`msg_type = Text | Reasoning` 且 `status = Streaming`）：登记节点 id，
///   全文转发（首片即完整快照）；
/// - 窄帧（`delta`）：只有**已登记的**节点才转发，按 id 落到它所属的那条流。
///
/// 为什么不直接转发所有 delta：model 插件的窄帧只带 `id + delta`，不带
/// `msg_type`——不记账就分不清正文增量、推理增量与工具参数增量。收束帧不经此桥：
/// 终态由消费方负责（model 插件只发 Streaming 快照与窄帧，见 `state.rs` 的收口）。
/// 工具参数增量**不进**这里：工具节点的构造权在分发方（`DispatchPort`），
/// 两处各建一份必然出现重复卡片。
struct DeltaBridge {
    sink: Arc<dyn DeltaSink>,
    /// 已登记的**增量节点** id → 它属于哪条流（正文 / 推理）。
    ///
    /// 窄帧只带 `id + delta`、不带 `msg_type`，归属只能查这张表。用**一张映射**
    /// 而不是两张集合：节点 id 唯一 ⇒「一个节点恰好属于一条流」是结构事实，
    /// 两张集合只能靠人工保证不重叠；而且窄帧是**每个 token 一次**的热路径，
    /// 两张集合要连开两次锁（一次判正文、一次判推理）。
    nodes: std::sync::Mutex<std::collections::HashMap<String, MessageType>>,
}

#[async_trait]
impl ExecTranscriptWriter for DeltaBridge {
    async fn apply(&self, m: ChatMessage) {
        if let Some(d) = &m.delta {
            if !d.is_empty() {
                // 窄帧不带 `msg_type` ⇒ 归属只能查登记表。没登记的 id（工具参数
                // 增量）落 `_` 臂：它们的构造权在分发方（见结构体文档）。
                match self.nodes.lock().unwrap().get(&m.id) {
                    Some(MessageType::Text) => self.sink.on_delta(d),
                    Some(MessageType::Reasoning) => self.sink.on_reasoning(d),
                    _ => {}
                }
            }
            return;
        }
        if m.status != Some(MessageStatus::Streaming) {
            return;
        }
        let Some(MessageContent::Text(t)) = &m.content else {
            return;
        };
        if t.is_empty() {
            return;
        }
        // 只登记**两条文本流**：工具参数增量不进这里（构造权在分发方）。
        let kind = match m.msg_type {
            Some(MessageType::Text) => MessageType::Text,
            Some(MessageType::Reasoning) => MessageType::Reasoning,
            _ => return,
        };
        self.nodes
            .lock()
            .unwrap()
            .insert(m.id.clone(), kind.clone());
        match kind {
            MessageType::Text => self.sink.on_delta(t),
            MessageType::Reasoning => self.sink.on_reasoning(t),
            _ => {}
        }
    }
}

/// ⑤ 端口的真实实现：把 `ModelProvider::execute_turn`（五态机 + SSE 解析）
/// 折进 `LlmAdapter::generate`（prompt 入、文本出）的最小适配。
pub struct ProviderLlmAdapter {
    provider: Arc<dyn ModelProvider>,
    /// 会话级系统提示词（full 档经 `for_turn` 注入；缺省 = 彩排用的通用提示）。
    system: Option<String>,
    /// 本轮的中止信号（full 档经 `for_turn` 注入；缺省自建——彩排无需中止）。
    abort: Option<ExecAbortSignal>,
}

impl ProviderLlmAdapter {
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        ProviderLlmAdapter {
            provider,
            system: None,
            abort: None,
        }
    }

    /// 会话轮构造：系统提示词 + 本轮中止信号直通——中止与提示词是**调用方的
    /// 语境**，适配器不再自作主张（此前硬编码通用提示 + 自建中止信号，
    /// full 档接管会话轮后两者都必须由 chat_loop 传入）。
    pub fn for_turn(
        provider: Arc<dyn ModelProvider>,
        system: &str,
        abort: ExecAbortSignal,
    ) -> Self {
        ProviderLlmAdapter {
            provider,
            system: Some(system.to_string()),
            abort: Some(abort),
        }
    }

    fn system_prompt(&self) -> &str {
        self.system
            .as_deref()
            .unwrap_or("You are a helpful assistant.")
    }

    fn abort_signal(&self) -> ExecAbortSignal {
        self.abort.clone().unwrap_or_default()
    }
}

#[async_trait]
impl LlmAdapter for ProviderLlmAdapter {
    fn model_id(&self) -> &str {
        self.provider.provider_id()
    }

    /// 单轮生成（委托流式路径 + 静默接收口——**同一条执行路径**，不为
    /// 「不要流式」造第二条）。
    async fn generate(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError> {
        let (text, _) = self
            .generate_streaming(tok, messages, Arc::new(SilentDeltas))
            .await?;
        Ok(text)
    }

    /// 流式生成：经帧桥把 SSE 增量逐片转给 `sink`；返回值 = 全文 + 实测耗时
    ///（收束语义与流式与否无关）。
    ///
    /// **无工具 = [`Self::generate_turn`] 的入参为空的同一路径**（`&[]`）：不另写
    /// 一条请求构造。「空文本」在这里是**失败**（调用方拿到 Ok(空串) 会把「模型没答」
    /// 当「答了空话」落成 final）——判定归这里，因为无工具轮没有别的产出可指望。
    async fn generate_streaming(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
        sink: Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let turn = self.generate_turn(tok, messages, &[], sink).await?;
        if turn.text.trim().is_empty() {
            // 空文本不是成功：调用方拿到 Ok(空串) 会把「模型没答」当「答了空话」
            // 落成 final——必须走兜底路径（I3），所以这里按失败返回。
            return Err(AdapterError::GenerationFailed(
                "model returned empty text".into(),
            ));
        }
        Ok((turn.text, turn.cost_ms))
    }

    /// 工具通道：工具清单下行 + 工具调用上行。
    ///
    /// **空文本在工具轮上是合法的**（纯工具调用轮不说话）——所以这里不做空文本
    /// 判定，判定归「本轮收束」的那一侧（`TurnRunner::run_with_tools`）：只有
    /// 「无工具调用 + 空文本」才是失败。
    ///
    /// ## 这里**曾经**把整段 prompt 包成一条 user 消息（ADR-048a 判定的退步）
    ///
    /// ```ignore
    /// let message = ChatMessage { role: Some(MessageRole::User), content: …(prompt) };
    /// execute_turn(self.system_prompt(), &[message], tools, …)
    /// ```
    ///
    /// 后果不是「排版难看」：`role: tool` 在**到达 provider 之前**就被拍成了
    /// user 消息里的纯文本，于是模型（a）看不到哪些正文是工具结果、`(b)` 拿不到
    /// `tool_call_id`、`(c)` 会把工具输出当成用户说过的话。而
    /// `execute_turn` 的签名本来就是 `(system_prompt, messages: &[ChatMessage], …)`
    /// ——它要的就是消息数组，是我们自己压扁了再传。
    async fn generate_turn(
        &self,
        _tok: &FullModel,
        messages: &[PromptMessage],
        tools: &[CapabilityMeta],
        sink: Arc<dyn DeltaSink>,
    ) -> Result<LlmTurn, AdapterError> {
        let started = std::time::Instant::now();
        // core 的结构化消息 → provider 的线格式。**逐条映射，不合并**。
        //
        // ## 工具轮为什么不是「一条 assistant + 一条 tool」就够
        //
        // `ModelProvider` 侧把消息按**树**展平（`message_builder::flatten_chat_messages`）：
        // 工具调用是 `ToolCall` **子节点**（参数在它自己的 `content` 里、工具名在
        // `name`），工具结果是它的 `Text(Tool)` **子节点**。展平末尾还有一道清洗：
        // **丢弃「无对应 tool_call 的 tool 结果」**——provider 要求 `role: tool` 的
        // `tool_call_id` 必须匹配前文某条 assistant 的 `tool_calls`。
        //
        // 所以若只发「assistant 正文写着『调用工具 x』」+「tool 结果」，
        // **所有 tool 结果都成孤儿被丢** ⇒ 模型永远收不到结果 ⇒ **无限工具循环**
        // （实测 9905 次请求、CLI 撞 120s 超时，症状完全指不到这一层）。
        //
        // 结论：**v2 也得按那棵树发**。这不是「新协议」，是同一协议的两种说法——
        // core 说「这是一次工具调用 + 这是它的结果」，adapter 译成节点结构。
        // 本次请求里声明过的**全部调用 id**——判「某条 tool 结果是本轮的（走树形）
        // 还是跨轮的（降级成上下文）」的**唯一**依据。
        //
        // ⚠️ 不能用「这条消息自己带没带 `tool_calls`」来判：调用与结果是**两条独立
        // 消息**，结果那条的 `tool_calls` 当然是 `None`——于是续写轮（恢复的工具调用
        // 在同一次请求里声明、结果也在这同一次）会被误判成「跨轮」而降级，
        // t27 报「恢复轮请求应带上恢复结果，且是 role=tool 消息（实际: 降级后的
        // assistant 文本）」。
        // 同一次调用**只把最终那条结果**发给模型（协议侧「一次调用一条结果」的约束）。
        //
        // ## 为什么要在 adapter 收口，而不是改协议
        //
        // 协议展平时对每个 ToolCall 节点只找**一条**结果（
        // `message_builder::find_tool_result` 返回 `Option`），于是同一次调用的多条结果里
        // 只有第一条会到模型：等待轮落「等用户」的契约说明、恢复轮落回填后的答案，
        // 模型只看到前者 —— t27 报「恢复轮请求应带上恢复结果」。
        //
        // ## 为什么「取最后一条」是对的，而不是「都发」或「都不发」
        //
        // - **都发**：协议不接（一条调用只能配一条结果）。
        // - **都不发**：模型看不到结果，会重复调用同一个工具。
        // - **取最后一条**：中间态（pending）不是模型的行动依据，**终态**才是。
        //   append-only 的**事实网格**照样保留全部结果（N3 不允许改已落事实），
        //   这里只是「送进模型的那一批」的选取。
        //
        // ⚠️ 选取按**出现顺序**（网格 append-only ⇒ 事件序），不是按内容启发式。
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

        let mut wire: Vec<ChatMessage> = Vec::with_capacity(messages.len() * 2);
        for (idx, m) in messages.iter().enumerate() {
            // 同一次调用只留**最终**那条结果（见上面 `last_result_of` 的理由）。
            // 在主循环里跳过，于是树形路径与降级路径都自动只看到终态 ——
            // 判据放一处，两条路径不会走散。
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
                // ── assistant 且带调用声明 → 造「Turn 根 + ToolCall 子节点」────
                ("assistant", Some(calls)) if !calls.is_empty() => {
                    let root_id = llm_short_id();
                    let mut root = ChatMessage {
                        id: root_id.clone(),
                        role: Some(MessageRole::Assistant),
                        content: Some(MessageContent::Text(m.text.clone())),
                        ..Default::default()
                    };
                    root.msg_type = Some(MessageType::Turn);
                    let mut nodes: Vec<ChatMessage> = vec![root];
                    for c in calls {
                        nodes.push(ChatMessage {
                            id: llm_short_id(),
                            parent_id: Some(root_id.clone()),
                            role: Some(MessageRole::Assistant),
                            msg_type: Some(MessageType::ToolCall),
                            name: Some(c.name.clone()),
                            // 参数走 content（展平时从这里 re-parse）——与 v1 落库
                            // 时的规范化口径一致（`tc.arguments.to_string()`）。
                            content: Some(MessageContent::Text(c.arguments.to_string())),
                            tool_call_id: Some(c.id.clone()),
                            ..Default::default()
                        });
                    }
                    // 结果挂到**对应的 ToolCall 节点**下（按 id 配对）。
                    //
                    // ⚠️ 只挂**最终那条**（见 `last_result_of`）：同一次调用的
                    // pending 与回填答案共用一个 id，而协议展平对每个 ToolCall 只取
                    // **第一条**结果（`find_tool_result` 返回 `Option`）。全挂上去的
                    // 话模型看到的是「等用户」而不是答案（t27 报「恢复轮请求应带上
                    // 恢复结果」）。
                    for r in messages
                        .iter()
                        .enumerate()
                        .filter(|(_, x)| x.role == "tool")
                        .filter(|(_, x)| {
                            calls.iter().any(|c| Some(&c.id) == x.tool_call_id.as_ref())
                        })
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
                        nodes.push(ChatMessage {
                            id: llm_short_id(),
                            parent_id: nodes
                                .iter()
                                .find(|n| n.tool_call_id == r.tool_call_id)
                                .map(|n| n.id.clone()),
                            role: Some(MessageRole::Tool),
                            msg_type: Some(MessageType::Text),
                            content: Some(MessageContent::Text(r.text.clone())),
                            tool_call_id: r.tool_call_id.clone(),
                            ..Default::default()
                        });
                    }
                    wire.extend(nodes);
                }
                // ── 历史工具结果（跨轮）：**降级成 assistant 文本**，不当 tool 消息 ──
                //
                // ## 为什么不能当 `role: tool` 发
                //
                // 协议要求 `role: tool` 消息的 `tool_call_id` 必须匹配**本次请求里**
                // 某条 assistant 的 `tool_calls`。而跨轮结果的调用发生在**上一轮的请求**
                // 里——本次请求里没有那条调用，于是它是孤儿，会被请求包清洗丢掉
                // （`message_builder` 末尾：丢「无对应 tool_call 的 tool 结果」）。
                //
                // 实测症状：t38 报「轮 1 请求应带**第一轮**的工具结果（实际: []）」——
                // 工具结果在**事实网格里**，投影也产出了，但**一个字节都没到模型**。
                // 而这正是本用例的主题（跨轮投影消费）。
                //
                // ## 所以降级成什么
                //
                // **assistant 消息，正文写清「这是哪次工具的结果」**。理由：
                //
                // - 模型跨轮需要的只是「我之前调用过什么、拿到了什么」——那是**上下文**，
                //   不需要协议级的配对；
                // - 降级不是丢信息：工具名与结果正文都在正文里；
                // - 标成 assistant 而不是 user，因为它**确实是本主体做过的事**，
                //   标成 user 就是撒谎（这与 ADR-048a 判定的「拍平成 user 消息」同一个病）。
                //
                // ⚠️ 唯一的损失是 `tool_call_id` 的配对能力——而**本轮**的调用与结果
                // 仍走上面的树形路径，配对能力完好。跨轮结果的配对本来就无从谈起
                // （上一轮的 id 在本次请求里没有对应物）。
                ("tool", _) if m.tool_calls.is_none() => {
                    // 「已嵌套」判据必须**连内容一起比**。
                    //
                    // 只比 `tool_call_id` 会把**同一次调用的两条结果**吃掉一条：
                    // 等待轮落一条「等用户」的结果、恢复轮落一条「回填后的答案」，
                    // 两条共用同一个调用 id（那就是同一次调用）——于是第二条被当
                    // 「已嵌套」跳过，模型只看到「等用户」而看不到答案。
                    // 实测症状：t27 报「恢复轮请求应带上恢复结果，且是 role=tool
                    // 消息」，而请求里只有那条 pending 结果。
                    //
                    // ⚠️ 这也意味着**树形路径里同样只能挂一条**（它按 id 找父节点、
                    // 逐条 push，所以多条都挂得下）——那一侧没问题，问题只在这去重。
                    let already_nested = wire.iter().any(|n| {
                        n.msg_type == Some(MessageType::Text)
                            && n.tool_call_id == m.tool_call_id
                            && n.role == Some(MessageRole::Tool)
                            && n.content.as_ref().map(|c| c.to_text()) == Some(m.text.clone())
                    });
                    // 本次请求里**有**它的调用声明 ⇒ 它已经在上面的树形路径里挂好了，
                    // 这里什么都不用做（`already_nested` 为真）。
                    let declared_here = m
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|id| declared_call_ids.contains(id));
                    if !already_nested && !declared_here {
                        wire.push(ChatMessage {
                            id: llm_short_id(),
                            role: Some(MessageRole::Assistant),
                            content: Some(MessageContent::Text(format!(
                                "（此前调用工具 {} 的结果）{}",
                                m.tool.as_deref().unwrap_or("<unnamed>"),
                                m.text
                            ))),
                            ..Default::default()
                        });
                    }
                }
                // ── 其余（user / 无调用的 assistant / 投影出的 tool）─────────
                _ => {
                    wire.push(ChatMessage {
                        id: llm_short_id(),
                        role: Some(match m.role.as_str() {
                            "assistant" => MessageRole::Assistant,
                            "tool" => MessageRole::Tool,
                            "system" => MessageRole::System,
                            // 未知角色**不**默认为 user：那会把「我不知道这是什么」
                            // 伪装成「这是用户说的话」。落到 `User` 是本仓的兜底约定
                            // （投影只产出三种角色 + 一条请求级 system），写明它是为了让改动的人看见这条
                            // 边界。
                            _ => MessageRole::User,
                        }),
                        content: Some(MessageContent::Text(m.text.clone())),
                        tool_call_id: m.tool_call_id.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        // 流式出口：转写帧经桥转成正文增量（桥不落盘——帧面只进回调）；
        // 中止信号直通调用方（会话轮的中止语义不因换执行路径而丢）。
        // 工具调用帧不进 v2 文本面：工具节点的构造权在分发方（`DispatchPort` 实现），
        // 由它按**同一份** `tool_calls` 建节点——两处各建一份必然出现重复卡片。
        let env = ExecEnv::new(
            ExecEventSink::direct(Arc::new(DeltaBridge {
                sink,
                nodes: Default::default(),
            })),
            self.abort_signal(),
        );
        let output = self
            .provider
            .execute_turn(self.system_prompt(), &wire, tools, &llm_short_id(), &env)
            .await
            .map_err(|e| match e {
                // 中止不压成失败：调用方据此**不落兜底格**（ADR-044 同源纪律）。
                PluginError::Aborted => AdapterError::Aborted,
                other => AdapterError::GenerationFailed(format!("{other}")),
            })?;
        Ok(LlmTurn {
            text: output.text,
            tool_calls: output.tool_calls,
            cost_ms: started.elapsed().as_millis() as u64,
        })
    }
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/adapters/provider_adapter.rs::apply: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
