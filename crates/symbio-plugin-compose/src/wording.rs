//! 答话生成 —— `Answered { reason: from_context }` 的那条产线（一次**静默** LLM 往返）。
//!
//! ## 为什么这一条必须生成而不是查模板
//!
//! `from_context` 的含义是「答案已经在对话里了」（用户问的是"我们刚才聊了什么"
//! 这类问题）。那段文本只能从对话线**组织**出来——模板给不了。这与 `classify` 的
//! 快速档判出 `from_context` 是同一条事实的两端：判决说"能答"，措辞就得真的答。
//!
//! ## 为什么出口是 `SilentDeltas`
//!
//! 与 `classify` 的分类请求同一条理由：这是**内部请求**，它的流式帧绝不能以自有身份
//! 进入对话流（否则每答一次就在前端留下一个空 Turn 骨架）。调用经
//! [`ProviderLlmAdapter`] 的端口面，静默是入参形态而不是这里自搭的出口
//! （全仓模型消费一条链，见 [D-106](../../symbio-plugin-model/README.md#d-106-模型消费端口归一内部请求共享-llmadapter未识别协议不静默兜底 "crates/symbio-plugin-model/src/README")）。
//! 可见的那句话由 `session` 用生成结果**单独写一个节点**——`compose` 自己一个节点都不写
//! （唯一写入者不变，D-020）。
//!
//! ## 为什么系统提示词要拼上注册段
//!
//! 生成的这句话是**助手本人**在对用户说话，人格（`setting`）与记忆（`session` /
//! `memory`）必须参与——否则同一个人在两处口吻不同。注册段经
//! `capability_prompts()` 取（与 session 取的是同一条通道、同一组键，各取各的，
//! 不共享状态）。
//!
//! ## 失败方向：退回模板
//!
//! 生成失败（没有模型服务 / 请求失败 / 空文本）返回 `None`，调用方（`plugin.rs`）
//! 随即落模板。**不给用户空白**：答话是用户唯一能看到的东西，空白等于这轮什么都没发生。

use std::sync::Arc;

use super::config::ComposeConfig;
use symbio_plugin_api::chat_node::{ChatNode, NodeRole};
use symbio_plugin_api::{
    capability_model_resolve, capability_prompts, llm_short_id, ExecAbortSignal, LlmAdapter,
    PluginInvokeRequest, PluginInvokeRequestExt, PromptMessage, ProviderLlmAdapter, Registry,
    SilentDeltas, TokenIssuer, CAPABILITY_VISITOR,
};
use symbio_plugin_api::{ComposeRequest, WordingReply};

/// 生成请求的**指令段**（拼在注册的系统提示词之后；可被
/// `ComposeConfig::instruction` 覆盖）。
///
/// 三条约束各自对应一种已知的坏输出：① 不复述问题（模型很爱把问题抄一遍再答）；
/// ② 不编（"我们刚才聊了什么"最容易诱发幻觉）；③ 不用工具（本插件没有工具，
/// 说"我去查一下"就是撒谎）。
///
/// 它是 `pub(crate)` 而不是私有：配置的文档与测试要能引用"出厂那段长什么样"，
/// 而不必再抄一遍字面量。
pub(crate) const INSTRUCTION: &str = "\
你是这个助手本人，正在和用户对话。用一两句话回答用户最后那句话。

只依据下面这段对话里**已经发生过的事**回答；不要调用任何工具，不要编造没发生过的事。
直接给答案，不要复述用户的问题，不要写「根据我们的对话记录」这类元话语。
如果对话里确实没有答案，就直说不知道，并建议用户换个说法。";

/// 生成一段答话（含**这次生成自己的思考事实**）。
///
/// 模型与指令段都来自**本插件自己的配置**（`config.rs`）：配置了 `model` 就用它
/// （答话是用户逐字读的那段文本，值得用比判决更好的模型），取不到落回会话选定的那个；
/// 配置了 `instruction` 就覆盖内置那段。
///
/// 思考内容（`reasoning` / `reasoning_id`）随答话一起带出：这是**一次真实的模型
/// 往返**，它的思考与正文同源——此前在函数出口被丢弃，`Reason` 节点对普通答话轮
/// 从未存在过（见 `WordingReply` 的文档）。
///
/// 返回 `None` = 生成不了（没有可用的模型服务 / 请求失败 / 空文本 / 对话线里没有
/// 用户发言）——调用方据此**退回模板**。
pub(crate) async fn generate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    req: &ComposeRequest,
    config: &ComposeConfig,
) -> Option<WordingReply> {
    // 先做**不花任何代价**的判据，再去找模型服务：没有"要回答的那句话"时，
    // 生成只会得到一句凭空的话——那时连能力访问器都不必碰。
    let messages = build_messages(&req.context);
    if !messages.iter().any(|m| m.role == "user") {
        symbio_plugin_api::plugin_debug!("compose", "[Compose] 对话线里没有用户发言，生成路径跳过");
        return None;
    }

    // 模型服务从**调用方带来的能力访问器**里取。优先本插件配置的那个 provider id
    // （model 插件在 traverse 广播里把**全部已启用**的 provider 一并交来），
    // 取不到就落回当次生效的那个——降级而不失效。
    let visitor: Arc<dyn Registry> = ctx.get(CAPABILITY_VISITOR)?;
    let provider = capability_model_resolve(&*visitor, config.model_id()).await?;

    // 选了哪个模型记一笔：配置的 id 写错时**没有错误信号**（行为是"用会话的模型"，
    // 一切照常），这行日志是唯一的线索。
    symbio_plugin_api::plugin_debug!(
        "compose",
        "[Compose] 答话使用模型 '{}'（配置 model={:?}）",
        provider.provider_id(),
        config.model_id()
    );

    let system_prompt = build_system_prompt(&visitor, config).await;
    // 中止 = 调用方若给了就跟着走，没给就是一个永不中止的独立信号（`route()` 直呼
    // 没有编排层，也就没有中止来源——见 `ExecAbortSignal::of`）。
    let llm = ProviderLlmAdapter::for_turn(provider, &system_prompt, ExecAbortSignal::of(&**ctx));
    let tok = TokenIssuer::issue_deep();

    // 端口面给整轮产物（正文 / 思考 / 实测耗时同一次响应），内部请求的出口是
    // `SilentDeltas`——不产生任何可见帧。
    match llm
        .generate_turn(&tok, &messages, &[], Arc::new(SilentDeltas))
        .await
    {
        Ok(turn) => {
            // cost_ms 与对话轮同一条实测口径（adapter 边界）。内部请求本身不写事件，
            // 但这次往返的耗时随 `WordingReply.cost_ms` 交回 `session`，由它落进
            // 本轮收束格的 `cost_ms`——台账仍只读落格事件（D-044），不会因为
            // 「请求是静默的」就少记一整类轮次的成本。
            symbio_plugin_api::plugin_debug!(
                "compose",
                "[Compose] 答话往返完成（cost_ms={}）",
                turn.cost_ms
            );
            let text = non_empty(&turn.text)?;
            // 思考事实与正文同源：有正文才可能有思考（正文空 = 整轮退模板）。
            let reasoning = non_empty(&turn.reasoning);
            // 静默请求没有「节点 id」可带（端口只回文本）：身份由这里铸一个，
            // 供 `session` 复刻 Reason 节点。它是一次新事实的身份，不是模型协议字段。
            let reasoning_id = reasoning.as_ref().map(|_| llm_short_id());
            Some(WordingReply {
                text,
                reasoning_id,
                reasoning,
                cost_ms: turn.cost_ms,
            })
        }
        Err(e) => {
            // 生成失败**不是**本轮的失败：调用方退回模板，用户照旧收到一句答话。
            // 因此 warn 而不 error——把它报成错误会让一次限流看起来像会话故障。
            symbio_plugin_api::plugin_warn!("compose", "[Compose] 答话生成失败，退回模板：{e}");
            None
        }
    }
}

/// 拼系统提示词：**注册段在前、指令段在后**。
///
/// 顺序不是随意的：注册段是"我是谁 / 我知道什么"（人格与记忆），指令段是"这一句
/// 该怎么写"。把指令放在后面，它才压得住长人格段里的措辞习惯。
///
/// 可覆盖的只有**指令段**：注册段不属于本插件（它由 `setting` / `session` /
/// `memory` 各自注册），在这里也拿不到"改它"的资格。
async fn build_system_prompt(visitor: &Arc<dyn Registry>, config: &ComposeConfig) -> String {
    let mut parts: Vec<String> = capability_prompts(&**visitor)
        .await
        .into_iter()
        .map(|(_, prompt)| prompt)
        .filter(|p| !p.trim().is_empty())
        .collect();
    parts.push(
        config
            .instruction_override()
            .unwrap_or(INSTRUCTION)
            .to_string(),
    );
    parts.join("\n\n")
}

/// 组生成请求的消息列表。
///
/// 直接用对话线投影：它就是一组 `role = user | assistant` 的文本节点，与端口的消息
/// 形态同构——不需要在这里重拼一段「以下是历史」的散文（那会把同一份事实变成两种
/// 形状，且给"谁说了什么"再错一次的机会）。
///
/// 空节点剔除：投影只保证"是文本节点"，不保证"有正文"。
fn build_messages(context: &[ChatNode]) -> Vec<PromptMessage> {
    context
        .iter()
        .filter(|m| !text_of(m).trim().is_empty())
        .map(prompt_message)
        .collect()
}

/// 对话线节点 → 端口的消息形态（与 `classify` 同一条映射；工具身份照带，
/// `tool_calls` 留空——`ChatNode` 的调用声明是树上的子节点，单条消息装不下）。
fn prompt_message(m: &ChatNode) -> PromptMessage {
    PromptMessage {
        role: role_word(m.node_role()).to_string(),
        text: text_of(m),
        tool_call_id: m.tool_call_id.clone(),
        tool: m.tool_name().map(str::to_string),
        tool_calls: None,
    }
}

/// 角色 → 端口词形（与 `PromptMessage::role` 的取值同一批词）。
fn role_word(role: Option<NodeRole>) -> &'static str {
    match role {
        Some(NodeRole::Assistant) => "assistant",
        Some(NodeRole::Tool) => "tool",
        Some(NodeRole::System) => "system",
        // 未知角色不伪造成 assistant / tool：落 user 是本仓的兜底约定（投影只产出
        // 三种角色 + 一条请求级 system，走到这里说明调用方给了非常规节点）。
        _ => "user",
    }
}

/// 只接受**有内容**的文本：空串（或纯空白）等于"这轮什么都没说"，
/// 它比退回模板更糟——用户会看到一条空消息。
fn non_empty(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

fn text_of(m: &ChatNode) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

#[cfg(test)]
#[path = "wording.test.rs"]
mod tests;
