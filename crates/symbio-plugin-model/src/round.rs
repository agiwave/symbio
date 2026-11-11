//! 一次模型往返的**产物与判决**：`RoundOutput` / `RoundToolCallInfo` /
//! `ModelFinishReason` / `ModelUsage` / `ModelSpend`。
//!
//! ## 为什么在本插件，而不在 `symbio-sdk`（`D-439`）
//!
//! `D-438` 把这五个类型下沉到了 `symbio_sdk::llm`，理由是「**两侧**共用：model 产、
//! loop / session 销」。那条理由来自 `port.model` **之前**的形状——那时会话侧直接持有
//! `RoundOutput`（`is_reasoning_only` / `effective_text` / `into_messages` 三个读方法
//! 甚至以 `impl RoundOutput` 的形式住在 loop 的 `message_build.rs`）。
//!
//! 端口落地之后读者消失了，形态上能查证：
//!
//! - 全仓**没有** `impl RoundOutput`，三个读方法连同它们的调用点一起消失；
//! - `loop` / `session` 的生产代码**零处**提到这五个名字——会话侧读的是端口执行体
//!   落下的**事实载荷键**（`usage` / `spend` / `reason` / `truncated`），见
//!   `crates/symbio-plugin-model/src/ledger.rs`（两本账随写方走，`D-461`）；
//! - 剩下的读者全在本插件：`stream.rs` 造、`bound_provider.rs` 填、`port.rs` 折成事实、
//!   `tool_accumulator.rs` 累积、四个协议适配归一化 `finish`。
//!
//! 于是它不属于「跨插件词汇」，而属于**本插件的私有形状**：端口那一侧交给环的是
//! 事实（JSON），不是 Rust 类型。按 `D-023` 的依赖方数量判据，一个插件独有的形状
//! 就该住在那个插件里——架构层只留**请求**词汇（`PromptMessage` 由 `classify` 构造、
//! `tool` 分发、`model` 消费，三处跨插件，仍在 `symbio_sdk::llm`）。
//!
//! ## 键名是契约，类型不是
//!
//! 跨侧真正共享的是 `port.model` 落下的**键名**与它们的含义（`usage` / `spend` /
//! `reason`），那由端口行契约声明、由消费方按 JSON 读。把类型抬到共享层并不会让
//! 这条契约更稳，只会让「改一个私有字段」看起来像改架构。

use serde_json::Value;

/// 流结束原因（由各协议的 `finish_reason` / `stop_reason` / `finishReason` 归一化）
///
/// ## 为什么必须有它
///
/// 若不区分结束原因，流结束一律被当成"正常完成"。当模型因 `max_tokens` 用尽而停在
/// `Length` 时：
/// 1. 断在半句的文本会被当完整回复呈现；
/// 2. 若截断发生在 `tool_calls` 的参数 JSON 中间，工具调用永远收集不完 →
///    `tools_done` 为空 → 循环按"无工具调用"正常退出——**用户看到的就是"对话突然结束"**。
///
/// 只有拿到 `ModelFinishReason`，收束判据才能区分"自然结束"与"被长度截断"。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelFinishReason {
    /// 自然结束
    #[default]
    Stop,
    /// 输出达到长度上限被截断（`finish_reason=length` / `stop_reason=max_tokens` / `MAX_TOKENS`）
    Length,
    /// 因需要调用工具而结束
    ToolCalls,
    /// 被内容安全策略过滤
    ContentFilter,
    /// 未识别的原因（保留原始字符串，便于排查兼容网关）
    Other(String),
}

impl ModelFinishReason {
    /// 从 provider 的原始字符串归一化。`None` 视为自然结束。
    pub fn from_provider(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim()).unwrap_or("") {
            "" | "stop" | "end_turn" | "STOP" | "stop_sequence" | "completed" => {
                ModelFinishReason::Stop
            }
            "length" | "max_tokens" | "MAX_TOKENS" | "max_output_tokens" | "incomplete" => {
                ModelFinishReason::Length
            }
            "tool_calls" | "tool_use" | "function_call" => ModelFinishReason::ToolCalls,
            "content_filter" | "SAFETY" | "RECITATION" | "refusal" => {
                ModelFinishReason::ContentFilter
            }
            other => ModelFinishReason::Other(other.to_string()),
        }
    }

    pub fn is_length(&self) -> bool {
        matches!(self, ModelFinishReason::Length)
    }
}

/// 单次请求的用量（用于校准 token 估算；provider 不一定给，故全部可选）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelUsage {
    pub input: Option<u32>,
    pub output: Option<u32>,
}

/// 一次模型往返的**钱**（价格 × 实测用量，在 adapter 边界折好）。
///
/// ## 为什么是整数纳单位，而不是浮点金额
///
/// 金额会一路累加（一本账、一份实例聚合）：浮点相加不满足结合律，同一批事实换个
/// 扫描顺序就能读出最后一分的差别。整数没有这个问题——投影只做 `u64` 累加，
/// 同一份切片永远同一本账（N1）。折算时的浮点只出现一次（写事实之前），
/// 落格之后账是整数。
///
/// ## 为什么币种跟着金额走
///
/// 价格是**配置**（用户可以换供应商、指向自建端点），币种由配置说，不能在这里
/// 猜或换算。不同币种的金额没有可加性，账本因此按币种分桶（见本插件的
/// `ledger.rs`：金额按币种分桶、桶内按主体摊开）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSpend {
    /// 金额，单位 1e-9 个货币单位（价格 × token 一次性折算后四舍五入）。
    pub amount_nano: u64,
    /// 币种代码（配置原样归一成大写；仅作标签，不换算汇率）。
    pub currency: String,
}

/// 一条工具调用的**结果形态**（累积过程在 [`crate::tool_accumulator`]）。
#[derive(Debug, Clone)]
pub struct RoundToolCallInfo {
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
    /// 由执行侧拒绝执行并回报明确错误。
    ///
    /// `None` 且 `arguments == {}` 是合法的：无参工具的空串/纯空白参数。
    pub parse_error: Option<String>,
    /// 非空 = 本调用因超出**单响应工具调用数上限**被系统丢弃（未执行）。
    ///
    /// 文本即反馈给模型的**工具结果正文**（流层生成，执行层原样落成结果子节点）。
    /// 存在理由：超限的处置必须是**反馈**而不是整轮失败——模型必须从工具结果里
    /// 读到「为什么后面的调用没了、下一步该怎么办」；整轮 `Err` 会让模型对发生
    /// 了什么一无所知，表现为流程无故中断（`6cf9520d` 死循环守卫的反馈面，
    /// 检测面在 [`crate::stream`] 的 `ToolCallDelta` 分支）。
    pub overflow_notice: Option<String>,
}

/// 一轮 LLM 请求的**产物**（`port.model` 执行体内部把它折成事实）。
///
/// **只装结果，不装过程**：工具调用的分片累积是 [`crate::tool_accumulator`] 的事，
/// 收口后才以 [`Self::tool_calls`] 的形态出现。于是「一轮请求产出了什么」这个形状里
/// 不含任何状态机。
#[derive(Default)]
pub struct RoundOutput {
    pub text: String,
    pub reasoning: String,
    pub response_id: Option<String>,
    /// 本轮完成的工具调用（**结果形态**）。
    pub tool_calls: Vec<RoundToolCallInfo>,
    /// 响应正文子节点的 short id（delta 更新期间保持一致）
    pub response_text_child_id: String,
    /// 思考子节点的 short id
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
