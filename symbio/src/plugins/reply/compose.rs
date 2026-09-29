//! 答话生成 —— `Answered { reason: from_context }` 的那条产线（一次**静默** LLM 往返）。
//!
//! ## 为什么这一条必须生成而不是查模板
//!
//! `from_context` 的含义是「答案已经在对话里了」（用户问的是"我们刚才聊了什么"
//! 这类问题）。那段文本只能从对话线**组织**出来——模板给不了。这与 `triage` 的
//! 快速档判出 `from_context` 是同一条事实的两端：判决说"能答"，措辞就得真的答。
//!
//! ## 为什么出口是 `silent()`
//!
//! 与 `triage` 的分类请求同一条理由：这是**内部请求**，它的流式帧绝不能以自有身份
//! 进入对话流（否则每答一次就在前端留下一个空 Turn 骨架）。可见的那句话由 `session`
//! 用生成结果**单独写一个节点**——`reply` 自己一个节点都不写（唯一写入者不变，ADR-020）。
//!
//! ## 为什么系统提示词要拼上注册段
//!
//! 生成的这句话是**助手本人**在对用户说话，人格（`setting`）与记忆（`session` /
//! `memory`）必须参与——否则同一个人在两处口吻不同。注册段经
//! `list_system_prompts()` 取（与 session 取的是同一条通道、同一组键，各取各的，
//! 不共享状态）。
//!
//! ## 失败方向：退回模板
//!
//! 生成失败（没有模型服务 / 请求失败 / 空文本）返回 `None`，调用方（`plugin.rs`）
//! 随即落模板。**不给用户空白**：答话是用户唯一能看到的东西，空白等于这轮什么都没发生。

use std::sync::Arc;

use crate::symbio_core::schemas::dialog::ComposeRequest;
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole};
use crate::symbio_core::{
    llm_short_id, CapabilityVisitor, ExecAbortSignal, ExecEnv, ExecEventSink, ModelProvider,
    PluginInvokeRequest, PluginInvokeRequestExt, CAPABILITY_VISITOR,
};

/// 生成请求的**指令段**（拼在注册的系统提示词之后）。
///
/// 三条约束各自对应一种已知的坏输出：① 不复述问题（模型很爱把问题抄一遍再答）；
/// ② 不编（"我们刚才聊了什么"最容易诱发幻觉）；③ 不用工具（本插件没有工具，
/// 说"我去查一下"就是撒谎）。
const INSTRUCTION: &str = "\
你是这个助手本人，正在和用户对话。用一两句话回答用户最后那句话。

只依据下面这段对话里**已经发生过的事**回答；不要调用任何工具，不要编造没发生过的事。
直接给答案，不要复述用户的问题，不要写「根据我们的对话记录」这类元话语。
如果对话里确实没有答案，就直说不知道，并建议用户换个说法。";

/// 生成一段答话。
///
/// 返回 `None` = 生成不了（没有可用的模型服务 / 请求失败 / 空文本 / 对话线里没有
/// 用户发言）——调用方据此**退回模板**。
pub(crate) async fn generate(
    ctx: &Arc<dyn PluginInvokeRequest>,
    req: &ComposeRequest,
) -> Option<String> {
    // 先做**不花任何代价**的判据，再去找模型服务：没有"要回答的那句话"时，
    // 生成只会得到一句凭空的话——那时连能力访问器都不必碰。
    let messages = build_messages(&req.context);
    if !messages.iter().any(|m| m.role == Some(MessageRole::User)) {
        crate::plugin_debug!("reply", "[Reply] 对话线里没有用户发言，生成路径跳过");
        return None;
    }

    // 模型服务从**调用方带来的能力访问器**里取（model 插件在 traverse 广播中注册的
    // 唯一生效实例）。取不到 ⇒ 生成不了，调用方退回模板。
    let visitor: Arc<dyn CapabilityVisitor> = ctx.get(CAPABILITY_VISITOR)?;
    let provider: Arc<dyn ModelProvider> = visitor.get_model_provider().await?;

    let system_prompt = build_system_prompt(&visitor).await;
    // 出口 = 静默（内部请求）；中止 = 调用方若给了就跟着走，没给就是一个永不中止的
    // 独立信号（`route()` 直呼没有编排层，也就没有中止来源——见 `ExecAbortSignal::of`）。
    let env = ExecEnv::new(ExecEventSink::silent(), ExecAbortSignal::of(&**ctx));
    let root_id = llm_short_id();

    match provider
        .execute_turn(&system_prompt, &messages, &[], &root_id, &env)
        .await
    {
        Ok(out) => non_empty(&out.text),
        Err(e) => {
            // 生成失败**不是**本轮的失败：调用方退回模板，用户照旧收到一句答话。
            // 因此 warn 而不 error——把它报成错误会让一次限流看起来像会话故障。
            crate::plugin_warn!("reply", "[Reply] 答话生成失败，退回模板：{e}");
            None
        }
    }
}

/// 拼系统提示词：**注册段在前、指令段在后**。
///
/// 顺序不是随意的：注册段是"我是谁 / 我知道什么"（人格与记忆），指令段是"这一句
/// 该怎么写"。把指令放在后面，它才压得住长人格段里的措辞习惯。
async fn build_system_prompt(visitor: &Arc<dyn CapabilityVisitor>) -> String {
    let mut parts: Vec<String> = visitor
        .list_system_prompts()
        .await
        .into_iter()
        .map(|(_, prompt)| prompt)
        .filter(|p| !p.trim().is_empty())
        .collect();
    parts.push(INSTRUCTION.to_string());
    parts.join("\n\n")
}

/// 组生成请求的消息列表。
///
/// 直接用对话线投影：它就是一组 `role = user | assistant` 的文本节点，与模型的消息
/// 形态同构——不需要在这里重拼一段「以下是历史」的散文（那会把同一份事实变成两种
/// 形状，且给"谁说了什么"再错一次的机会）。
///
/// 空节点剔除：投影只保证"是文本节点"，不保证"有正文"。
fn build_messages(context: &[ChatMessage]) -> Vec<ChatMessage> {
    context
        .iter()
        .filter(|m| !text_of(m).trim().is_empty())
        .cloned()
        .collect()
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

fn text_of(m: &ChatMessage) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

#[cfg(test)]
#[path = "compose.test.rs"]
mod tests;
