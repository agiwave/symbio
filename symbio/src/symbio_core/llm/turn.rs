//! 单轮 LLM 产物与**共享帧原语**（core `llm/` 契约层的共享面）。
//!
//! 职责（协议无关、插件无关）：
//! - 单轮产物（[`TurnOutput`]）：`ModelProvider::execute_turn` 的返回类型
//!   （见 [`super::model_provider`]）——**只装结果，不装过程**
//! - 工具调用信息（[`TurnToolCallInfo`]）：[`TurnOutput::tool_calls`] 的元素类型
//! - 共享帧原语：[`llm_emit_message`]（完整消息）与 [`llm_removed_frame`]（删除）
//!   ——**两个以上模块**共用的帧构造 / 发射点
//! - id 原语 [`llm_short_id`]：消息节点 id 的统一格式（流式累积与落库共用）
//!

use super::model_provider::{ModelFinishReason, ModelUsage};
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageStatus};
use crate::symbio_core::ExecEventSink;
use serde_json::Value;

// 执行期出口（共享的完整消息帧）

/// 发送一条**完整消息**（`content` = 整条替换，幂等）。
///
/// 用在正文对接收端是**新的权威副本**的帧上：一次性节点（工具结果 / 用户消息
/// 回填）的单帧完成、存储回执、压缩快照。流式节点的正文已由 `llm_emit_delta`
/// （`plugins/model/stream.rs`）逐帧传过，它的终态走 `llm_emit_state`
/// （`plugins/session/frames.rs`），不在这里重发。
pub async fn llm_emit_message(sink: &ExecEventSink, msg: ChatMessage) {
    sink.emit(llm_message_frame(&msg)).await;
}

/// 由一条完整消息派生**消息帧**（`content` = 整条替换）：缺省补 `completed`。
///
/// 直接写转写（`Transcript::apply`，不经出口）的发布路径也用它——「完整消息必然
/// 带状态」这条约定只在这里实现一次。
pub fn llm_message_frame(m: &ChatMessage) -> ChatMessage {
    let mut frame = m.clone();
    if frame.status.is_none() {
        frame.status = Some(MessageStatus::Completed);
    }
    frame
}

/// 删除帧：`status = removed`（发射方与收口路径的唯一构造点，避免各写一份）。
pub fn llm_removed_frame(message_id: &str) -> ChatMessage {
    ChatMessage {
        id: message_id.to_string(),
        status: Some(MessageStatus::Removed),
        ..Default::default()
    }
}

// 工具调用信息（**结果形态**——累积过程在 `plugins/model/tool_accumulator.rs`）

/// Tool call information
#[derive(Debug, Clone)]
pub struct TurnToolCallInfo {
    /// 消息节点 id（会话内唯一）：流式帧 / 落库 / 工具结果锚定都用它。
    pub id: Option<String>,
    /// provider 原始 `tool_call_id`（wire id）。`None` = 供应商未提供，
    /// 请求构建回退节点 id（历史上节点 id 就是 wire id，旧数据天然成立）。
    pub wire_id: Option<String>,
    pub name: Option<String>,
    pub arguments: Value,
    /// 参数 JSON **非空且解析失败**时的原始文本；其余情况为 `None`。
    ///
    /// 存在理由：累积器早先对解析失败静默回退 `{}`，于是「参数被
    /// max_tokens 截断、参数残破」与「无参工具的空参数」在下游长得一模一样——
    /// 工具收到空参后报「缺少必填参数」，模型误以为调用合法而原样重试，
    /// 形成卡思考死循环。此字段把「解析失败」这一事实显式携带到执行侧，
    /// 由 `process_tool_calls_async` 拒绝执行并回报明确错误。
    ///
    /// `None` 且 `arguments == {}` 是合法的：无参工具的空串/纯空白参数。
    pub parse_error: Option<String>,
}

// id 原语（消息节点 id 的统一格式，两侧共用）

/// 生成长度短的 ID（8 字符，取 UUID v4 前缀）
pub fn llm_short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

// 单轮产物

/// 一轮 LLM 请求的**产物**（`ModelProvider::execute_turn` 的返回类型）。
///
/// **只装结果，不装过程**：工具调用的分片累积是 model 插件的实现细节
/// （`plugins/model/tool_accumulator.rs`），收口后才以 [`Self::tool_calls`] 的形态
/// 交给 session。于是「一轮请求产出了什么」这个契约面里不含任何状态机。
///
/// **三个读方法不在本模块**：`is_reasoning_only` / `effective_text` /
/// `into_messages`（落库视图）定义在 `plugins/session/message_build.rs`——
/// 它们的外部消费方只有 session，且 `into_messages` 依赖那侧的
/// `llm_build_assistant_messages`。固有实现可落在同一 crate 的任意模块，
/// 调用方不需要 import 实现所在处。
#[derive(Default)]
pub struct TurnOutput {
    pub text: String,
    pub reasoning: String,
    pub response_id: Option<String>,
    /// 本轮完成的工具调用（**结果形态**）。
    ///
    /// `is_reasoning_only` / `effective_text` / `into_messages` 一律读这里，
    /// 不再由调用方另行传 `n_tools`——「有几个工具」与「是哪些工具」是同一
    /// 件事，分两处传入迟早会不一致。
    pub tool_calls: Vec<TurnToolCallInfo>,
    /// Short ID for the response text child node (consistent across delta updates)
    pub response_text_child_id: String,
    /// Short ID for the reasoning child node
    pub reasoning_child_id: String,
    /// 流结束原因（一次响应最多一次）。用于区分「自然结束」与「max_tokens 截断」
    /// （不区分即表现为「对话突然结束」）。默认 Stop。
    pub finish: ModelFinishReason,
    /// 用量统计（provider 不一定给，故可选）。用于校准 token 估算器。
    pub usage: Option<ModelUsage>,
}
