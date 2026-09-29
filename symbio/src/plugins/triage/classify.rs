//! 快速档分类 —— 一次**静默** LLM 往返，四选一。
//!
//! ## 为什么是「静默」而不是「一次正常请求」
//!
//! 分类请求是**内部请求**：它的流式帧（Turn 骨架 / 思考 / 正文增量）绝不能以自有
//! 身份进入对话流。否则每判一次就在前端留下一个永不 finalize 的空 Turn——
//! 长会话下逐个累积，且只在"每轮都判"之后才复现，极易误判成渲染层问题。
//!
//! `ExecEventSink::silent()` 的文档写明它「用途唯一且明确——内部请求」，与上下文
//! 压缩摘要同款。判决与压缩摘要是**同一形状**：内部模型调用 + 不产生可见节点。
//! 因此这里没有新机制，只是这条既有通道的第二个使用者。
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

use std::sync::Arc;

use crate::symbio_core::schemas::dialog::Verdict;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole,
};
use crate::symbio_core::{
    llm_short_id, CapabilityVisitor, ExecAbortSignal, ExecEnv, ExecEventSink, ModelProvider,
    PluginInvokeRequest, PluginInvokeRequestExt, CAPABILITY_VISITOR,
};

use super::reasons::{REASON_CLARIFY, REASON_FROM_CONTEXT, REASON_NEEDS_WORK, REASON_REFUSE};

/// 分类器的系统提示词。
///
/// 它必须**只输出一个词**——本模块的解析（[`parse_verdict`]）建立在这一点上。
/// 让模型输出 JSON 或一句话再解析，等于把「这轮走哪条路」变成一次不可靠的字符串
/// 匹配（`schemas::dialog` 的模块文档写了同一条理由）。
const SYSTEM_PROMPT: &str = "\
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
/// 返回 `None` = 判不出来（没有可用的模型服务 / 请求失败 / 响应不可解析）——
/// 调用方据此落 `Escalate`（见模块文档的失败方向说明）。
pub(crate) async fn classify(
    ctx: &Arc<dyn PluginInvokeRequest>,
    utterance: &str,
    context: &[ChatMessage],
) -> Option<Verdict> {
    // 模型服务从**调用方带来的能力访问器**里取（model 插件在 traverse 广播中注册的
    // 唯一生效实例）。取不到 ⇒ 判不出来，调用方落 `Escalate`。
    let visitor: Arc<dyn CapabilityVisitor> = ctx.get(CAPABILITY_VISITOR)?;
    let provider: Arc<dyn ModelProvider> = visitor.get_model_provider().await?;

    let messages = build_messages(utterance, context);
    // 出口 = 静默（内部请求）；中止 = 调用方若给了就跟着走，没给就是一个永不中止的
    // 独立信号（`route()` 直呼没有编排层，也就没有中止来源——见 `ExecAbortSignal::of`）。
    let env = ExecEnv::new(ExecEventSink::silent(), ExecAbortSignal::of(&**ctx));
    let root_id = llm_short_id();

    match provider
        .execute_turn(SYSTEM_PROMPT, &messages, &[], &root_id, &env)
        .await
    {
        Ok(out) => parse_verdict(&out.text),
        Err(e) => {
            // 分类失败**不是**本轮的失败：它只让判决退回 `Escalate`（= 今天的行为）。
            // 因此这里 warn 而不 error——把它报成错误会让一次限流看起来像会话故障。
            crate::plugin_warn!(
                "triage",
                "[Triage] 快速档分类请求失败，按 Escalate 处理：{e}"
            );
            None
        }
    }
}

/// 组分类请求的消息列表。
///
/// 直接用对话线投影：它就是一组 `role = user | assistant` 的文本节点，与模型的
/// 消息形态同构——不需要在这里重新拼一段「以下是历史」的散文（那会把同一份事实
/// 变成两种形状，且让「谁说了什么」在提示词里再错一次的机会）。
fn build_messages(utterance: &str, context: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = context
        .iter()
        .filter(|m| !text_of(m).trim().is_empty())
        .cloned()
        .collect();

    // 投影已含本轮这句话时不再追加（正常路径：`session` 先把用户消息折进上下文再判）。
    let tail_matches = messages
        .last()
        .map(|m| m.role == Some(MessageRole::User) && text_of(m) == utterance)
        .unwrap_or(false);
    if !tail_matches {
        messages.push(user_message(utterance));
    }
    messages
}

fn user_message(text: &str) -> ChatMessage {
    ChatMessage {
        id: llm_short_id(),
        role: Some(MessageRole::User),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn text_of(m: &ChatMessage) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

#[cfg(test)]
#[path = "classify.test.rs"]
mod tests;
