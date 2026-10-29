//! 单轮 LLM 产物的**类型面**（`ModelProvider::round_trip` 的返回类型）。
//!
//! 帧构造 / 发射与 id 原语是宿主行为，留在 `symbio_core::llm::turn`。

use super::model_provider::{ModelFinishReason, ModelSpend, ModelUsage};
use serde_json::Value;

// 工具调用信息（**结果形态**——累积过程在 `crates/symbio-plugin-model/src/tool_accumulator.rs`）

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
    /// 非空 = 本调用因超出**单响应工具调用数上限**被系统丢弃（未执行）。
    ///
    /// 文本即反馈给模型的**工具结果正文**（流层生成，执行层原样落成结果子节点）。
    /// 存在理由：超限的处置必须是**反馈**而不是整轮失败——模型必须从工具结果里
    /// 读到「为什么后面的调用没了、下一步该怎么办」；整轮 `Err` 会让模型对发生
    /// 了什么一无所知，表现为流程无故中断（`6cf9520d` 死循环守卫的反馈面，
    /// 检测面在 `crates/symbio-plugin-model/src/stream.rs` 的 `ToolCallDelta` 分支）。
    pub overflow_notice: Option<String>,
}

// 单轮产物

/// 一轮 LLM 请求的**产物**（`ModelProvider::round_trip` 的返回类型）。
///
/// **只装结果，不装过程**：工具调用的分片累积是 model 插件的实现细节
/// （`crates/symbio-plugin-model/src/tool_accumulator.rs`），收口后才以 [`Self::tool_calls`] 的形态
/// 交给 session。于是「一轮请求产出了什么」这个契约面里不含任何状态机。
///
/// **三个读方法不在本模块**：`is_reasoning_only` / `effective_text` /
/// `into_messages`（落库视图）定义在 `crates/symbio-plugin-loop/src/message_build.rs`——
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
    /// 这一次往返的**钱**（[`ModelSpend`]）：价格 × 实测用量，由实现体在
    /// adapter 边界折好。`None` = 没有价格配置或没有实测用量——**两种都不写零账**
    /// （「没定价」与「明确免费」由 `Some(amount_nano: 0)` 分辨）。
    pub spend: Option<ModelSpend>,
}
