//! `adapters` —— 外部资源的唯一接缝（v2 ⑤，[plan/05 §3.3](../../../../docs/plan/05-模块架构.md)）。
//!
//! ## 为什么令牌住这里
//!
//! 「能不能碰模型」本质是「能不能拿到外部资源句柄」——归属 ⑤ 是唯一的正确位置
//! （[plan/05 §3.3](../../../../docs/plan/05-模块架构.md)）。时延闸门不是新机制，
//! 就是本包的**依赖注入策略**：`assemble` / 令牌签发按档位给证，调用方拿到的
//! 令牌类型决定它能调哪些方法——**反射档超时不是被检测到，而是不可能发生**。
//!
//! ## 契约出处
//!
//! 形状逐字来自可执行论证
//! [`docs/plan/verify/latency_gate.rs`](../../../../docs/plan/verify/latency_gate.rs)；
//! 四层预算表来自 [plan/01 §10](../../../../docs/plan/01-核心架构.md)。
//!
//! ## 「依赖」的边界（诚实划界）
//!
//! 令牌是 **ZST 契约类型**（零大小、零运行时开销、私有构造不可伪造）。
//! ② `actors` 的 `Reasoner` 方法签名引用 `FullModel`——这是注入策略的**类型边**，
//! 不是机制依赖：它不携带任何 ⑤ 的行为，只携带「该主体被装配在深度档」这一事实。

use async_trait::async_trait;

use crate::symbio_core::chat_message::ChatMessage;
use crate::symbio_core::ModelUsage;
use crate::symbio_core::PromptMessage;
use crate::symbio_core::{CapabilityMeta, TurnToolCallInfo};

// ── 四层时延（[plan/01 §10](../../../../docs/plan/01-核心架构.md)，一等契约） ──

/// 时延档位。**判据是时延，不是形式**——约束的是 `budget_ms`，不是"工具"这个名词。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LatencyTier {
    /// 反射：≤80ms，纯规则/查表，**禁模型**（对应人类通路：脊髓反射）。
    Reflex,
    /// 快速：~300ms，只允许**分类**不允许生成（视觉简单反应）。
    Fast,
    /// 深度：秒–分钟，完整规划 + 工具（复杂决策）。
    Deep,
    /// 自主：小时–天，常驻、事件驱动。
    Autonomic,
}

impl LatencyTier {
    /// 四层预算（[plan/01 §8](../../../../docs/plan/01-核心架构.md) 参数表）。
    pub fn budget_ms(self) -> u64 {
        match self {
            LatencyTier::Reflex => 80,
            LatencyTier::Fast => 300,
            LatencyTier::Deep => 60_000,
            LatencyTier::Autonomic => 86_400_000,
        }
    }

    /// 档位名（入事件的 `tier` 字段——兜底率按层统计的口径锚）。
    pub fn name(self) -> &'static str {
        match self {
            LatencyTier::Reflex => "reflex",
            LatencyTier::Fast => "fast",
            LatencyTier::Deep => "deep",
            LatencyTier::Autonomic => "autonomic",
        }
    }

    /// 从事件载荷的 `tier` 字符串还原档位（未知/缺失 ⇒ `None`——**不静默归类**，
    /// 投影侧把 None 计入可观测的 `unspecified` 桶）。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "reflex" => Some(LatencyTier::Reflex),
            "fast" => Some(LatencyTier::Fast),
            "deep" => Some(LatencyTier::Deep),
            "autonomic" => Some(LatencyTier::Autonomic),
            _ => None,
        }
    }

    /// 该档位是否允许**触碰模型**。
    pub fn may_call_model(self) -> bool {
        matches!(
            self,
            LatencyTier::Fast | LatencyTier::Deep | LatencyTier::Autonomic
        )
    }

    /// 该档位是否允许**生成**（而非只分类）。
    pub fn may_generate(self) -> bool {
        matches!(self, LatencyTier::Deep | LatencyTier::Autonomic)
    }
}

// ── 能力令牌：三档 ZST，私有构造 ⇒ 不可伪造 ─────────────────────────────

/// 反射档令牌：证明持码者处于「禁模型」档。
pub struct RuleOnly {
    _private: (),
}

/// 快速档令牌：允许**分类**（输出空间受限），不允许生成。
pub struct ClassifyOnly {
    _private: (),
}

/// 深度档令牌：允许完整模型调用 + 工具。
pub struct FullModel {
    _private: (),
}

impl RuleOnly {
    fn mint() -> Self {
        RuleOnly { _private: () }
    }
}
impl ClassifyOnly {
    fn mint() -> Self {
        ClassifyOnly { _private: () }
    }
}
impl FullModel {
    fn mint() -> Self {
        FullModel { _private: () }
    }
}

// ── 令牌层级：trait 表达「高档能做低档的事」 ────────────────────────────
//
// 这两只 trait 与下面三条 impl 只服务**编译期正/负例**（`adapters/mod.test.rs`：
// `FullModel` 满足 `CanClassify`、`RuleOnly` 不满足）——生产侧的同一道闸门
// 由签名本身表达（`LlmAdapter::generate` 只收 `FullModel`），不需要运行时问它。
// 故整组留在 `#[cfg(test)]`：进测试构建，不占生产面。

/// 能做分类（快速档 / 深度档 / 自主档都满足）。
#[cfg(test)]
pub trait CanClassify {}
/// 能完整生成（仅深度档 / 自主档）。
#[cfg(test)]
pub trait CanGenerate: CanClassify {}

#[cfg(test)]
impl CanClassify for ClassifyOnly {}
#[cfg(test)]
impl CanClassify for FullModel {}
#[cfg(test)]
impl CanGenerate for FullModel {}
// 注意：`RuleOnly` **不**实现 `CanClassify`——这就是闸门在类型层的全部表达。

/// 令牌签发中心：**唯一**能造令牌的地方，按档位签发（一个 match，零运行时开销）。
pub struct TokenIssuer;

impl TokenIssuer {
    pub fn issue_reflex() -> RuleOnly {
        RuleOnly::mint()
    }
    pub fn issue_fast() -> ClassifyOnly {
        ClassifyOnly::mint()
    }
    pub fn issue_deep() -> FullModel {
        FullModel::mint()
    }
}

// ── LLM 端口：只接受 `FullModel` 令牌 ──────────────────────────────────

/// 适配失败的形态（可扩展：超时 / 限流 / 协议错误在真实接线时按需增臂）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// 生成失败（模型返回错误 / 结果不可用）。**调用方必须产出兜底事件**（I3）。
    GenerationFailed(String),
    /// 生成被中止（用户主动停止 / 会话销毁）。
    ///
    /// **调用方不得产出兜底事件**——中止不是失败：轮未收束，网格少一格是
    /// 诚实的缺口（ADR-044 的转写纪律同源），也绝不能被计入兜底（兜底率是
    /// 失败的指标：用户按的停止不是「模型答不出」）。中止与失败的差别必须
    /// **在类型上**可见：压成一个变体，消费方就只能靠错误文本猜（会话结局
    /// `aborted` 与 `failed` 的分派即由此定）。
    Aborted,
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::GenerationFailed(why) => write!(f, "生成失败：{why}"),
            AdapterError::Aborted => write!(f, "已中止"),
        }
    }
}

/// 一轮生成的**完整**产物（[`LlmAdapter::generate_turn`] 的返回类型）。
///
/// ## 为什么不是 `(String, u64)` 再加一条旁路
///
/// 工具调用与正文是**同一次响应**的两面（模型要么答话、要么要工具），分两处返回
/// 必然要回答"哪个是权威"——而答案是两个都不是：**一次响应 = 一个产物**。
/// `tool_calls` 复用 core 既有的 [`TurnToolCallInfo`]：那是 provider 边界上已在用的
/// 类型，另立一个必然要写映射，两处形状必然漂移。
#[derive(Debug, Clone, Default)]
pub struct LlmTurn {
    /// 本轮正文。纯工具调用轮为空——「没说话」与「没答」是两件事（后者是失败）。
    pub text: String,
    /// 模型**请求**的工具调用（尚未执行）；空 = 本轮收束（落 final）。
    pub tool_calls: Vec<TurnToolCallInfo>,
    /// adapter 边界实测耗时（毫秒）——与 [`LlmAdapter::generate_timed`] 同一口径。
    pub cost_ms: u64,
    /// provider **实测用量**（本轮这一次响应的）。与 [`Self::cost_ms`] 同族：
    /// 都是 adapter 边界上已经看在眼里的实测数，只是观察对象不同（耗时 vs token）。
    ///
    /// 为什么必须带出来：唯一消费方是 session 插件的 token 估算校准
    /// （`chat_loop/turn.rs` 的 `feedback_estimate`）——provider 返回的真实
    /// `output_tokens` 是校准比的分子，断在这里校准就永远停在初值 1.0，
    /// 未校准启发式的系统偏差（实测对 CJK 高估约 31%）永久无人修正，
    /// 压缩预检会把本可成功的摘要请求误判成「注定超限」（缺口 6）。
    ///
    /// 缺省 `None`：provider 不一定给（协议或桩都不保证有），校准侧本就按
    /// 可选处理——**缺是正常的，断链才是缺陷**。
    pub usage: Option<ModelUsage>,
}

/// 流式增量接收口（[`LlmAdapter::generate_streaming`] 的回调面）。
///
/// `Send + Sync + 'static`——适配器在自己的执行任务上逐片回调，回调体可能
/// 跨线程投递（如 [`crate::symbio_core::exec::ExecEventSink`] 的桥接实现）。
pub trait DeltaSink: Send + Sync + 'static {
    /// 正文增量（按序追加；适配器保证语义为「追加到正文尾部」）。
    fn on_delta(&self, text: &str);
    /// 推理增量（`reasoning_content`——模型在正文之前的思考）。
    ///
    /// 与 [`Self::on_delta`] **分属两条流**：消费端各自建节点（`msg_type` 不同），
    /// 既不合并也不互相追加。合成一条会让「模型在想什么」与「模型说了什么」在节点上
    /// 再也分不开，而这两件事在界面与事实网格里都是分开的（v1 的 `ReasoningDelta`
    /// 走的就是两条子节点）。
    ///
    /// **必填而不是给个空默认实现**：漏实现的表现是「推理静默消失」——没有编译错误、
    /// 没有告警，只是界面上永远少一块。本仓对这类「静默失效」一律要求显式表态。
    fn on_reasoning(&self, text: &str);

    /// 工具调用帧——**消息形状**的第三条出口。
    ///
    /// 两种帧都经这里（与 v1 的 `stream.rs` 同构，快照 / 增量由帧自身字段区分）：
    /// - 快照（`delta = None`）：身份帧，`id` / `name` / `tool_call_id` / 参数至今
    ///   全在上面——delta 必须落在身份帧之后（前端没有渲染语义可挂）；
    /// - 窄增量（`delta = Some(片段)`）：只有 `id + delta`，接收端尾部拼接。
    ///
    /// ## 为什么是整条消息而不是拆开的参数
    ///
    /// **工具节点的构造权在分发方**（[`DispatchPort`]），不在这里：model 插件的
    /// 转写面已经用**同一个节点 id** 广播（流式 / 落库 / 执行三处同 id，见
    /// `tool_accumulator`），分发方稍后以同一 id 定格与落库。桥若在这里自建节点、
    /// 或把帧拆成裸参数再让消费端重组，就会出现第二张卡。所以消费端的义务只是
    /// **把帧原样送进自己的帧序**——不做类型推断，不改写身份。
    fn on_tool_frame(&self, frame: &ChatMessage);
}

/// 静默接收口：会话侧执行器（`TurnRunner`，非流式形态）的委托目标——同一条
/// 执行路径，不为「不要流式」造第二条。
pub struct SilentDeltas;

impl DeltaSink for SilentDeltas {
    fn on_delta(&self, _text: &str) {}
    fn on_reasoning(&self, _text: &str) {}
    fn on_tool_frame(&self, _frame: &ChatMessage) {}
}

/// LLM 端口（⑤ 的抽象面）。**generate 只接受 `FullModel`**——反射/快速档
/// 在类型上拿不到生成能力。
///
/// 实现方两个：生产是 `ProviderLlmAdapter`（包 `ModelProvider`，见同目录
/// `provider_adapter`），测试是 `#[cfg(test)]` 的零 LLM 桩 `StubLlmAdapter`。
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    /// 模型标识（入 final 事件的载荷，可观测）。
    fn model_id(&self) -> &str;
    /// 生成。`tok` 是闸门：没有 `FullModel` 就调不到这里。
    ///
    /// **入参是消息数组而不是一段文本**（[ADR-048a](../../../../docs/decisions/session.md)）：
    /// 角色与正文分开，工具结果保持 `role: "tool"`——`ModelProvider::execute_turn` 的签名
    /// `(system_prompt, messages: &[ChatMessage], …)` 本来就是消息数组；拍成一段文本会让
    /// `role` 在 adapter 边界被拍平，模型收到的是散文而不是消息序列。
    async fn generate(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError>;

    /// 埋点版生成（SLO 校准，[plan/04 §1](../../../../docs/plan/04-工程落地.md)）：
    /// 除文本外返回 **adapter 边界实测耗时**（毫秒）。默认实现包一层
    /// `Instant`——桩与真实适配器零改动共享；调用方把它写到 final 事件的
    /// `cost_ms`，成本台账（③ cost_ledger）与熔断判据从此有了真实来源
    /// （G3：参数有了实测数字）。
    async fn generate_timed(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<(String, u64), AdapterError> {
        let start = std::time::Instant::now();
        let out = self.generate(tok, messages).await?;
        Ok((out, start.elapsed().as_millis() as u64))
    }

    /// 流式生成（v2 执行路径的 UI 帧源）：正文逐片经 `sink.on_delta` 送出，
    /// 返回值与 [`Self::generate_timed`] 同形（全文 + 实测耗时——收束语义
    /// 与流式与否无关，final 事件照常落格）。
    ///
    /// 默认实现 = **一次性发全文**：非流式适配器的诚实降级——调用方收到的
    /// 帧少，但顺序、收束、记账全部不变。真实适配器（SSE）覆写本方法逐片回调。
    async fn generate_streaming(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let out = self.generate_timed(tok, messages).await?;
        sink.on_delta(&out.0);
        Ok(out)
    }

    /// **工具通道**：工具清单下行、工具调用上行（v2 运行器做工具轮的前提）。
    ///
    /// 默认实现 = **忽略工具清单、`tool_calls` 恒空**——这是**诚实降级**而非失败：
    /// 不支持工具的适配器照常答文本。没有这条通道，运行器在**类型上**就拿不到模型
    /// 请求的工具调用（见 `docs/plan/10-工具轮v2化实施方案.md` §1）。
    ///
    /// 与 [`Self::generate_streaming`] 的关系：无工具时两者等价（默认实现即经它委托）
    /// ——「有工具」与「无工具」不是两条执行路径，是同一条的两种入参。
    async fn generate_turn(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
        _tools: &[CapabilityMeta],
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<LlmTurn, AdapterError> {
        let (text, cost_ms) = self.generate_streaming(tok, messages, sink).await?;
        Ok(LlmTurn {
            text,
            tool_calls: Vec::new(),
            cost_ms,
            usage: None,
        })
    }
}

/// 一次工具调用的**事实形状**（[`DispatchPort::dispatch`] 的元素类型）。
///
/// 刻意只装**结果**：调用方（运行器）要的是「拿什么回填给模型」与「要不要停」，
/// 不是插件侧的节点形状。节点怎么建、往哪发，是分发方自己的事——core 不认识
/// `ChatMessage` 的图语义（那在 `plugins/session`），只认识这段文本。
#[derive(Debug, Clone, Default)]
pub struct DispatchOutcome {
    /// 对应的工具调用 id（回填给模型时锚定 ToolCall；取 `TurnToolCallInfo::id`）。
    pub call_id: String,
    /// 工具名（LLM 可见名）。
    pub name: String,
    /// 结果正文。**失败也是正文**——工具失败是信息性的，模型据此改道，不中断会话。
    pub text: String,
    /// 成功与否。仅供可观测/展示；**不改变**本轮终态（同 v1 的信息性策略）。
    pub ok: bool,
    /// 本次调用**收束于等待用户动作**（confirm / ask_user ⇒ 本轮停下等用户）。
    /// 运行器据此**停止工具循环**（恢复前提见
    /// [plan/11 批 2](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)）。
    pub needs_user_action: bool,
}

/// 工具分发端口（⑤ 的抽象面，与 [`LlmAdapter`] 并列的第二只端口）。
///
/// ## 为什么是 trait 而不是把分发搬进 core
///
/// 分发的**全部输入**都在插件侧：插件宿主（路由工具能力）、调用上下文（会话 id /
/// 工作区 / 能力注册表）、转写（结果节点写进对话图）、会话目录（超长结果存档）。
/// core 认识插件即违 E-009，而依赖方向由编译器保证——同 [`crate::symbio_core::ProviderLlmAdapter`]
/// 留在 core 的那条理由（它只依赖 core 契约）。
///
/// ## 为什么实现方持有中止信号
///
/// 与 [`LlmAdapter`] 同形：中止是**调用的语境**，由构造方注入实现体，不由每轮传参。
/// 分发方在批内逐工具检查 `abort.is_aborted()` 并提前收口（未执行的批尾必须定格，
/// 否则前端留下永远转动的「运行中」）。
#[async_trait]
pub trait DispatchPort: Send + Sync {
    /// 分发一轮的工具调用，返回每个调用的结果事实。
    ///
    /// `turn` 是本轮生成的完整产物——分发方需要它的 `text` 与 `tool_calls` 一起，
    /// 才能把这一轮**落成一条完整的助手消息**（正文 + 工具调用节点），而不是只落
    /// 工具结果（那样下一轮请求里模型看不到自己请求过什么）。
    async fn dispatch(&self, turn: &LlmTurn) -> Vec<DispatchOutcome>;
}

/// 零 LLM 桩——彩排与测试用。可注入**确定性失败**，用于演练兜底路径。
///
/// 真实适配器（包 `ModelProvider`）由 e2e 的 MockLlm / 真实 provider 接线承接。
/// 桩只有测试这一个消费面，故整组（结构 + 构造 + 实现 + 分片委托）进
/// `#[cfg(test)]`——不占生产面，也就没有「接入生产」这一步可走。
#[cfg(test)]
pub struct StubLlmAdapter {
    model: &'static str,
    /// 非空 ⇒ generate 一律返回该错误（演练兜底）。
    fail_with: Option<&'static str>,
    /// 注入的延迟毫秒（演练 cost_ms 埋点；测试配合 `tokio::time` 使用）。
    delay_ms: u64,
    /// 非空 ⇒ 逐片生成（演练流式回调；全文 = 各片拼接，与单发等价）。
    chunks: Option<Vec<String>>,
    /// 真 ⇒ 一律以 [`AdapterError::Aborted`] 返回（演练中止路径：不落兜底格）。
    abort: bool,
}

#[cfg(test)]
impl StubLlmAdapter {
    pub fn succeed(model: &'static str) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms: 0,
            chunks: None,
            abort: false,
        }
    }

    /// 中止桩：生成一律返回 [`AdapterError::Aborted`]（演练「中止不落兜底格」）。
    pub fn aborting(model: &'static str) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms: 0,
            chunks: None,
            abort: true,
        }
    }

    /// 逐片生成的桩：演练流式回调（`generate_streaming` 覆写逐片送出）。
    pub fn succeed_streaming(model: &'static str, chunks: &[&str]) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms: 0,
            chunks: Some(chunks.iter().map(|c| c.to_string()).collect()),
            abort: false,
        }
    }

    /// 一律失败的桩：演练「生成失败 → 兜底必须产生事件」。
    pub fn always_fail(message: &'static str) -> Self {
        StubLlmAdapter {
            model: "stub",
            fail_with: Some(message),
            delay_ms: 0,
            chunks: None,
            abort: false,
        }
    }

    /// 带注入延迟的桩（演练时延埋点）。
    pub fn with_delay(model: &'static str, delay_ms: u64) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms,
            chunks: None,
            abort: false,
        }
    }
}

#[cfg(test)]
#[async_trait]
impl LlmAdapter for StubLlmAdapter {
    fn model_id(&self) -> &str {
        self.model
    }

    async fn generate(
        &self,
        _tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError> {
        if self.abort {
            return Err(AdapterError::Aborted);
        }
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        match self.fail_with {
            Some(msg) => Err(AdapterError::GenerationFailed(msg.to_string())),
            None => match &self.chunks {
                // 分片桩：全文 = 各片拼接（与单发等价——流式只是帧的形态）。
                Some(chunks) => Ok(chunks.concat()),
                // 回显**末条正文**：旧签名是单段文本，所以这里就是全文；改成消息数组后
                // 若仍拼全部消息，桩的输出会随轮次线性变长，进而**改变既有断言的
                // 字面量**。取末条 = 与「模型只答最后一句」一致，也最贴近真实形态。
                None => Ok(format!(
                    "[{}] {}",
                    self.model,
                    messages.last().map(|m| m.text.as_str()).unwrap_or("")
                )),
            },
        }
    }

    async fn generate_streaming(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        if self.abort {
            return Err(AdapterError::Aborted);
        }
        let Some(chunks) = &self.chunks else {
            // 无分片配置 ⇒ 走默认降级（一次性全文）。
            return LlmAdapter::generate_streaming(&DelegateGen(self), tok, messages, sink).await;
        };
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if let Some(msg) = self.fail_with {
            return Err(AdapterError::GenerationFailed(msg.to_string()));
        }
        for c in chunks {
            sink.on_delta(c);
        }
        let full = chunks.concat();
        Ok((full, 0))
    }
}

/// 委托包装：让无分片的桩走进 trait 的**默认**实现（Rust 裸调用默认方法
/// 需要 `Self` 类型，包装避免把默认体复制一份）。
#[cfg(test)]
struct DelegateGen<'a>(&'a StubLlmAdapter);

#[cfg(test)]
#[async_trait]
impl LlmAdapter for DelegateGen<'_> {
    fn model_id(&self) -> &str {
        self.0.model_id()
    }
    async fn generate(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError> {
        self.0.generate(tok, messages).await
    }
}

mod provider_adapter;

pub use provider_adapter::ProviderLlmAdapter;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
