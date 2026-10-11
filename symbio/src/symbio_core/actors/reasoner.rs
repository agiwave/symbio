//! 生成档主体（`pattern = Reasoner`）与 prompt 的结构化渲染入口（[ADR-048a](../../../../../docs/decisions/session.md)）。

use crate::symbio_core::adapters::{AdapterError, FullModel, LlmAdapter};
use crate::symbio_core::Event;

/// 生成档主体（`pattern = Reasoner` 的 S2 形态，[plan/01 §4](../../../../../docs/plan/01-核心架构.md)）。
///
/// ## 闸门在签名上
///
/// [`Reasoner::reply`] 要求 `&FullModel` 令牌——**反射 / 快速档拿不到这个令牌**，
/// 所以「反射层调模型」不是被检测到，而是**编译不过**（J3，[plan/01 §10](../../../../../docs/plan/01-核心架构.md)
/// 第 4 条语义）。令牌由 ⑤ `adapters` 的 [`crate::symbio_core::adapters::TokenIssuer`]
/// 按档位签发，装配时注入；`Reasoner` 自身不持有任何适配器句柄。
pub struct Reasoner;

impl Reasoner {
    /// 生成答复：从事件切片取最后一条用户消息作输入，经端口生成。
    ///
    /// 失败形态是 [`AdapterError`]——**调用方必须产出 `chat.assistant.fallback` 事件**
    /// （I3 到点必答：禁止静默超时，[plan/01 §10](../../../../../docs/plan/01-核心架构.md) 第 2 条）。
    pub async fn reply(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
    ) -> Result<String, AdapterError> {
        Ok(self.reply_timed(llm, tok, events).await?.0)
    }

    /// 埋点版 reply（SLO 校准，[plan/04 §1](../../../../../docs/plan/04-工程落地.md)）：
    /// 除文本外返回 adapter 边界实测耗时（毫秒）。调用方把它写到 final 事件的
    /// `cost_ms`——成本台账与熔断判据（② CircuitBreaker）从此读**实测值**
    /// 而不是声明值（G3：成本模型有了实测数字）。
    pub async fn reply_timed(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
    ) -> Result<(String, u64), AdapterError> {
        self.reply_streaming(
            llm,
            tok,
            events,
            std::sync::Arc::new(crate::symbio_core::adapters::SilentDeltas),
        )
        .await
    }

    /// 流式版：生成增量逐片经 `sink` 送出（v2 执行路径的 UI 帧源）；
    /// 返回值与 [`Self::reply_timed`] 同形——流式只是帧的形态，收束语义不变。
    pub async fn reply_streaming(
        &self,
        llm: &dyn LlmAdapter,
        tok: &FullModel,
        events: &[Event],
        sink: std::sync::Arc<dyn crate::symbio_core::adapters::DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let messages = Self::render_messages(events);
        llm.generate_streaming(tok, &messages, sink).await
    }

    /// prompt 的**唯一渲染点**（[ADR-048a](../../../../../docs/decisions/session.md)）：
    /// 事件切片 → 转写投影 → **消息数组**。
    ///
    /// prompt 从**转写投影**出（多轮带历史）——历史来自同一份事实源，不另存副本
    /// （ADR-044 同族纪律）。**结构化是唯一出口**：拍成一段文本会把
    /// `role: tool` 降级成散文里的纯文本，模型就看不到「这是工具结果」、也拿不到
    /// `tool_call_id`——那是已判定的退步，不是排版偏好。
    ///
    /// ## 为什么运行器仍要就地追加工具交换
    ///
    /// 投影读的是**已落格**的事实（[plan/10 批 3](../../../../../docs/plan/10-工具轮v2化实施方案.md)
    /// 起，跨轮的工具结果也在其中）；而**本轮**刚发生的工具交换，快照取在它落格
    /// **之前**（`run_with_tools` 先取 `base_messages`、再逐轮追加），投影里当然没有。
    /// 所以运行器把这一轮的交换就地累积成 `exchange` 追加到基线上——**一条基线 +
    /// 就地累积**，而不是两条渲染路径。续写轮同理：恢复产生的交换也先落格再追加。
    pub fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
        crate::symbio_core::transcript()
            .apply(events, i64::MAX, crate::symbio_core::Budget::generous())
            .value
            .to_messages()
    }
}

/// prompt 的**结构化渲染入口**：事件切片 → 消息数组（[`Reasoner::render_messages`] 的一跳封装）。
///
/// 生产侧（下沉后的 `plugins/session/turn_runner`）只准调本函数，**不得在定义域之外提及
/// 主体类型**——`scripts/no-direct-call-audit.mjs` 的 NDC-001：提及即可持有、持有即可直连
/// （协作只走事件是 I1 的直接推论）。渲染本身是**纯投影**（不碰主体状态），缝只在
/// 「什么时候渲染、往基线上追加什么」由写方决定、「怎么把事件变成消息」由本域决定——
/// 与 [`recalled_event`] 同一形态（生产侧驱动运行器而不逐个持有主体）。
pub fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
    Reasoner::render_messages(events)
}
