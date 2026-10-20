//! 快速档分类 —— 一次**静默** LLM 往返，四选一。
//!
//! ## 为什么是「静默」而不是「一次正常请求」
//!
//! 分类请求是**内部请求**：它的流式帧（Turn 骨架 / 思考 / 正文增量）绝不能以自有
//! 身份进入对话流。否则每判一次就在前端留下一个永不 finalize 的空 Turn——
//! 长会话下逐个累积，且只在"每轮都判"之后才复现，极易误判成渲染层问题。
//!
//! 调用经 `ProviderLlmAdapter` 的端口面（`generate_turn` + `SilentDeltas`）：静默是
//! 端口的入参形态，不是这里自己搭的出口。判决与上下文压缩摘要是**同一形状**——
//! 内部模型调用 + 不产生可见节点；区别只在本条已经收敛到 `LlmAdapter`，压缩仍在
//! 直调（唯一登记豁免，见 [ADR-106](../../../../docs/010-adr/106.md "ADR-106")）。
//!
//! ## 为什么「拿不准」的兜底是 `Escalate`
//!
//! 分类失败（没有可用的模型服务 / 响应不可解析）必须有一个方向。选 `Escalate`
//! 的理由是失败方向：`Escalate` 退化成「今天的行为」（全部进工具循环，功能完整），
//! 而 `Answered` 会让一次分类故障表现成**用户什么都收不到**——静默，无信号。
//!
//! ## 为什么只认四个词
//!
//! 判决是**闭集枚举**，分类器的输出词汇表必须与它同宽：三个「能直接答」的理由码
//! 加一个「要干活」。多出来的类别（情绪 / 领域 / 优先级）没有对应的编排动作，
//! 收了也只能丢掉——那是给提示词加装饰，不是加能力。
//!
//! ## 为什么这次调用走深度档端口而**不持**快速档令牌
//!
//! 这里就是快速档本身，但它经 [`ProviderLlmAdapter`] + `TokenIssuer::issue_deep()`
//! 发一次静默往返，不拿 `ClassifyOnly`——[ADR-096](../../../../docs/010-adr/096.md "ADR-096")
//! 把那张令牌退役了。理由：它守的 `Classifier` 端口在生产里从未存在（全仓只在
//! `verify/latency_gate.rs` 里作为编译期桩），而「输出只能是四个词之一」由
//! [`parse_verdict`] 的返回类型强制，「不带工具」由本插件不写工具行强制
//! （`traverse_registers_no_tool_rows` 数注册表）。令牌在这里只能证明
//! 「持码者被允许分类」，那句证明不带来任何调用方会检查的东西。
//!
//! 走深度档端口不是「升档」：内部请求的形状（空工具、静默、调用方中止）没有变，
//! 变的是**取模型能力的路**——全仓只剩一条（[ADR-106](../../../../docs/010-adr/106.md "ADR-106")）。
//! 以前直调 `execute_turn` 的那条旁路同时绕开了闸门与 adapter 的 cost_ms 实测。

use std::sync::Arc;

use crate::symbio_core::chat_node::{ChatNode, NodeRole};
use crate::symbio_core::Verdict;
use crate::symbio_core::{
    capability_model_resolve, ExecAbortSignal, LlmAdapter, PluginInvokeRequest,
    PluginInvokeRequestExt, PromptMessage, ProviderLlmAdapter, Registry, SilentDeltas, TokenIssuer,
    CAPABILITY_VISITOR, REASON_CLARIFY, REASON_FROM_CONTEXT, REASON_NEEDS_WORK, REASON_REFUSE,
};

use super::config::ClassifyConfig;

/// 分类器的系统提示词（**内置那份**，可被 `ClassifyConfig::system_prompt` 覆盖）。
///
/// 它必须**只输出一个词**——本模块的解析（[`parse_verdict`]）建立在这一点上。
/// 让模型输出 JSON 或一句话再解析，等于把「这轮走哪条路」变成一次不可靠的字符串
/// 匹配（`schemas::dialog` 的模块文档写了同一条理由）。
///
/// 它是 `pub(crate)` 而不是私有：配置的文档与测试要能引用"出厂那份长什么样"，
/// 而不必再抄一遍字面量。
pub(crate) const SYSTEM_PROMPT: &str = "\
你是一个意图分流器。读用户最后这一句话，判断这一轮该怎么处理，只输出一个词：

direct  —— 用户在问已有对话里能回答的事实（或纯粹在寒暄确认），不需要动任何工具
clarify —— 用户的话含义不明，缺少关键信息，应当先反问一句
refuse  —— 用户在要求做不该做 / 做不到的事，应当明确拒绝
work    —— 其余情况：需要读文件、跑命令、查资料、改动任何东西，总之要动手

只输出上面四个词中的一个，不要标点、不要解释、不要 JSON。";

/// 四选一的词表：`(词, 判决)`。
///
/// 判决由**词**决定，不由提示词的措辞决定——改提示词不会改这里的行为。
const CHOICES: &[(&str, &str)] = &[
    ("direct", REASON_FROM_CONTEXT),
    ("clarify", REASON_CLARIFY),
    ("refuse", REASON_REFUSE),
    ("work", REASON_NEEDS_WORK),
];

/// 把分类器的自由文本解析成判决。
///
/// 两级：先看**首个非空行**归一化后是否逐字等于某个词（正常路径——提示词要求只输出
/// 一个词）；不成立再退化为「全文里最早出现的那个词」（模型多说了几句时仍能救回）。
/// 都不成立 ⇒ `None`（调用方落 `Escalate`）。
pub(crate) fn parse_verdict(text: &str) -> Option<Verdict> {
    let first_line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let exact = normalize_word(first_line);
    if let Some((_, reason)) = CHOICES.iter().find(|(word, _)| *word == exact) {
        return Some(verdict_of(reason));
    }

    let lowered = text.to_lowercase();
    CHOICES
        .iter()
        .filter_map(|(word, reason)| lowered.find(word).map(|at| (at, reason)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, reason)| verdict_of(reason))
}

/// 词 → 判决：`work` 是唯一的「要干活」，其余三个都是「能直接答」。
fn verdict_of(reason: &str) -> Verdict {
    if reason == REASON_NEEDS_WORK {
        Verdict::Escalate {
            reason: reason.to_string(),
        }
    } else {
        Verdict::Answered {
            reason: reason.to_string(),
        }
    }
}

/// 剥掉首尾标点与空白（模型常带个句号 / 引号回来）。
fn normalize_word(raw: &str) -> String {
    raw.to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// 快速档分类：一次静默 LLM 往返。
///
/// `context` 是**对话线**投影（`session` 经 `context::conversation_view` 投影后带来），
/// 最后一条即本轮用户发言。它为空（仓外调用方直呼本路由）时退回「只用这句话」。
///
/// 模型与提示词都来自**本插件自己的配置**（`config.rs`）：配置了 `model` 就用它
/// （判决是一次四选一分类，最便宜最快的模型就够），取不到落回会话选定的那个；
/// 配置了 `system_prompt` 就覆盖内置那份。
///
/// 返回 `None` = 判不出来（没有可用的模型服务 / 请求失败 / 响应不可解析）——
/// 调用方据此落 `Escalate`（见模块文档的失败方向说明）。
pub(crate) async fn decide(
    ctx: &Arc<dyn PluginInvokeRequest>,
    utterance: &str,
    context: &[ChatNode],
    config: &ClassifyConfig,
) -> Option<Verdict> {
    // 模型服务从**调用方带来的能力访问器**里取。优先本插件配置的那个 provider id
    // （model 插件在 traverse 广播里把**全部已启用**的 provider 一并交来），
    // 取不到就落回当次生效的那个——降级而不失效。
    let visitor: Arc<dyn Registry> = ctx.get(CAPABILITY_VISITOR)?;
    let provider = capability_model_resolve(&*visitor, config.model_id()).await?;

    // 选了哪个模型记一笔：配置的 id 写错时**没有错误信号**（行为是"用会话的模型"，
    // 一切照常），这行日志是唯一的线索。
    crate::plugin_debug!(
        "classify",
        "[Classify] 分类使用模型 '{}'（配置 model={:?}）",
        provider.provider_id(),
        config.model_id()
    );

    let system_prompt = config.prompt_override().unwrap_or(SYSTEM_PROMPT);
    let messages = build_messages(utterance, context);
    // 中止 = 调用方若给了就跟着走，没给就是一个永不中止的独立信号（`route()` 直呼
    // 没有编排层，也就没有中止来源——见 `ExecAbortSignal::of`）。
    let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, ExecAbortSignal::of(&**ctx));
    let tok = TokenIssuer::issue_deep();

    // 端口面直接给 `LlmTurn`：正文、实测耗时与用量是同一次响应的产物，不为「只要
    // 文本」再走一条降级路径。`SilentDeltas` 是内部请求的出口形态（不产生任何帧）。
    match llm
        .generate_turn(&tok, &messages, &[], Arc::new(SilentDeltas))
        .await
    {
        Ok(turn) => {
            // cost_ms 与对话轮同一条实测口径（adapter 边界）；内部请求不写事件，
            // 因此它进日志而不是成本台账（台账的唯一输入仍是落格事件，ADR-044）。
            crate::plugin_debug!(
                "classify",
                "[Classify] 分类往返完成（cost_ms={}）",
                turn.cost_ms
            );
            parse_verdict(&turn.text)
        }
        Err(e) => {
            // 分类失败**不是**本轮的失败：它只让判决退回 `Escalate`（= 今天的行为）。
            // 因此这里 warn 而不 error——把它报成错误会让一次限流看起来像会话故障。
            crate::plugin_warn!(
                "classify",
                "[Classify] 快速档分类请求失败，按 Escalate 处理：{e}"
            );
            None
        }
    }
}

/// 组分类请求的消息列表。
///
/// 直接用对话线投影：它就是一组 `role = user | assistant` 的文本节点，与端口的
/// 消息形态同构——不需要在这里重新拼一段「以下是历史」的散文（那会把同一份事实
/// 变成两种形状，且让「谁说了什么」在提示词里再错一次的机会）。
fn build_messages(utterance: &str, context: &[ChatNode]) -> Vec<PromptMessage> {
    let mut messages: Vec<PromptMessage> = context
        .iter()
        .filter(|m| !text_of(m).trim().is_empty())
        .map(prompt_message)
        .collect();

    // 投影已含本轮这句话时不再追加（正常路径：`session` 先把用户消息折进上下文再判）。
    let tail_matches = messages
        .last()
        .map(|m| m.role == "user" && m.text == utterance)
        .unwrap_or(false);
    if !tail_matches {
        messages.push(user_message(utterance));
    }
    messages
}

/// 对话线节点 → 端口的消息形态。
///
/// 角色词与 `NodeRole` 的序列化名逐字一致。工具身份照带（投影正常不产出工具节点，
/// 但契约允许仓外调用方直呼本路由）——`tool_calls` 留空：`ChatNode` 把调用声明表达为
/// 树上的子节点，单条消息装不下，硬造一份等于给同一次调用两个真相。
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

fn user_message(text: &str) -> PromptMessage {
    PromptMessage {
        role: "user".to_string(),
        text: text.to_string(),
        ..Default::default()
    }
}

fn text_of(m: &ChatNode) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

#[cfg(test)]
#[path = "decide.test.rs"]
mod tests;
