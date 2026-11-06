//! 答话生成的**提示词**（`wording.rs` 里不再有 provider 往返）。
//!
//! 生成这件事已经搬到环上：`debt.rs` 申报一行 `model.assist` 的口债，派发与三格
//! 由环写、执行体归 model 的端口。本文件只回答一个问题——**这一笔请求该带什么
//! 系统提示**。
//!
//! ## 为什么不再自己取注册段
//!
//! 注册段（人格 / 记忆）本来经 `capability_prompts()` 取；但债行的 `rows()` 是同步
//! 申报口，拿不到能力访问器。本轮已经把**同一份装配结果**落在 `session.config`
//! 的 `system_prompt` 上（它是模型端口本轮读的那条输入），措辞行直接读这一格——
//! 不另开一次段收集，也就不存在两份提示词漂移的可能（D-341 裁决三）。
//!
//! ## 顺序：注册段在前、指令段在后
//!
//! 注册段是"我是谁 / 我知道什么"，指令段是"这一句该怎么写"。把指令放在后面，
//! 它才压得住长人格段里的措辞习惯。可覆盖的只有**指令段**：注册段不属于本插件。

use super::config::ComposeConfig;

/// 生成请求的**指令段**（拼在已装配的系统提示词之后；可被
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

/// 拼这一笔 `model.assist` 请求的系统提示词：**注册段在前、指令段在后**。
///
/// `session_prompt` 是本轮 `session.config` 里那份已装配的系统提示词（人格 / 记忆
/// 与其它注册段都在里面）；`config` 只提供可覆盖的指令段。
pub(crate) fn system_prompt(config: &ComposeConfig, session_prompt: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !session_prompt.trim().is_empty() {
        parts.push(session_prompt);
    }
    parts.push(config.instruction_override().unwrap_or(INSTRUCTION));
    parts.join("\n\n")
}

#[cfg(test)]
#[path = "wording.test.rs"]
mod tests;
