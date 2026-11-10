//! 模型往返的**数据词汇**：结束原因、用量、花费。
//!
//! `D-437` 之后本模块**不含端口**。此前这里住着 `ModelProvider`——「能执行一次模型
//! 往返」的 trait，理由是「会话引擎与 model 插件之间唯一的模型契约」。那条理由在
//! `port.model` 落地后失效了：环只认端口名与 `PortExecutor`，模型怎么解析、怎么发、
//! 怎么解析流全在 model 插件内部，**没有任何一个插件在生产路径上需要命名那个 trait**
//! （它唯一的两个外部读口 `capability_model_active` / `capability_model_resolve`
//! 自建立起就没有生产调用方，另有一个 `ProviderLlmAdapter` 只被测试构造）。
//!
//! 留下来的三个类型形状不变，因为它们的读者是**两侧**：model 插件产出它们，
//! 会话侧读它们（`RoundOutput.finish` 判截断、`usage` 校准估算、`spend` 记账）。
//! 数据形状跨侧共用不构成「架构层认了一个世界」，那正是 `Event` / `ChatNode` 的处境。
//!
//! ## 谁还认这些词
//!
//! | 符号 | 谁依赖 | 依赖什么 |
//! |---|---|---|
//! | [`ModelFinishReason`] | model（`RoundOutput::finish` 的赋值方）· 投影（区分「自然结束」与 `max_tokens` 截断） | `from_provider` / `is_length` |
//! | [`ModelUsage`] | model（上报）· session（校准 token 估算） | 字段读取 |
//! | [`ModelSpend`] | model（折价）· session（记账） | 字段读取 |
//!
//! 端口形态（`round_trip` 的签名、五态机、限流与上下文收敛）随之归
//! `crates/symbio-plugin-model/src/provider.rs`——谁实现谁持有，扩展面不再有第二个
//! 答案（`D-253` 的 EX-008 豁免因此也没有对象了）。

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
/// 猜或换算。不同币种的金额没有可加性，账本因此按币种分桶（见 `spend_ledger`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSpend {
    /// 金额，单位 1e-9 个货币单位（价格 × token 一次性折算后四舍五入）。
    pub amount_nano: u64,
    /// 币种代码（配置原样归一成大写；仅作标签，不换算汇率）。
    pub currency: String,
}
