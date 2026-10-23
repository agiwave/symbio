//! 快速档分类 —— **四选一**的词表、提示词与解析。
//!
//! ## 为什么这里**不再打模型**
//!
//! D-122 决策 2：判决的模型往返走**引擎的 `model` 端口派发路径**——插件侧不再
//! 保留直接打模型的入口。于是本模块从「一次静默 LLM 往返」缩成三件**纯**事：
//!
//! 1. 规则表命中与否（判完 ⇒ 零往返，编排层直接收判决）；
//! 2. 没命中时备一份**计划**（提示词 + 消息 + 模型）交给编排层去打；
//! 3. 编排层把应答拿回来时，按四个词解析成判决。
//!
//! 「为什么是静默而不是一次正常请求」这条理由**没有变**，只是搬到了编排层：分类请求
//! 的流式帧绝不能以自有身份进入对话流（否则每判一次就在前端留下一个永不 finalize
//! 的空 Turn）。判据归谁管是另一件事——**分类请求的形状**（空工具、静默、调用方中止）
//! 由计划里的消息与编排层的出口形态保证。
//!
//! ## 为什么只认四个词
//!
//! 判决是**闭集枚举**，分类器的输出词汇表必须与它同宽：三个「能直接答」的理由码
//! 加一个「要干活」。多出来的类别（情绪 / 领域 / 优先级）没有对应的编排动作，
//! 收了也只能丢掉——那是给提示词加装饰，不是加能力。
//!
//! ## 为什么「拿不准」的兜底是 `Escalate`
//!
//! 分类失败（没有可用的模型服务 / 响应不可解析）必须有一个方向。选 `Escalate`
//! 的理由是失败方向：`Escalate` 退化成「今天的行为」（全部进工具循环，功能完整），
//! 而 `Answered` 会让一次分类故障表现成**用户什么都收不到**——静默，无信号。

use crate::symbio_core::chat_node::{ChatNode, NodeRole};
use crate::symbio_core::Verdict;
use crate::symbio_core::{
    DecisionPlan, PromptMessage, REASON_CLARIFY, REASON_FROM_CONTEXT, REASON_NEEDS_WORK,
    REASON_REFUSE,
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

/// 备好一次判决模型往返的**计划**（D-122 决策 2）。
///
/// `context` 是**对话线**投影（`session` 经 `context::conversation_view` 投影后带来），
/// 最后一条即本轮用户发言。它为空（仓外调用方直呼本路由）时退回「只用这句话」。
///
/// 提示词与模型都来自**本插件自己的配置**（`config.rs`）：配置了 `model` 就用它
/// （判决是一次四选一分类，最便宜最快的模型就够），取不到落回会话选定的那个；
/// 配置了 `system_prompt` 就覆盖内置那份。
///
/// ## 为什么计划里带**模型 id** 而不是带一个 provider
///
/// 契约跨插件，而 provider 是**运行期对象**（`Arc<dyn ModelProvider>`）——它过不了
/// 契约的序列化面。带 id 让编排层自己去解析（`capability_model_resolve`：严格查找 +
/// 单点降级），解析不到就整轮不落格——「没有模型服务」这一档因此仍是**零往返**。
pub(crate) fn plan(utterance: &str, context: &[ChatNode], config: &ClassifyConfig) -> DecisionPlan {
    DecisionPlan {
        system_prompt: config
            .prompt_override()
            .unwrap_or(SYSTEM_PROMPT)
            .to_string(),
        messages: build_messages(utterance, context),
        model_id: config.model_id().map(str::to_string),
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
