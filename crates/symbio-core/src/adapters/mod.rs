//! `adapters` —— 外部资源的唯一接缝（⑤，[DE-018 §3.3](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）。
//!
//! ## 为什么令牌住这里
//!
//! 「能不能碰模型」本质是「能不能拿到外部资源句柄」——归属 ⑤ 是唯一的正确位置
//! （[DE-018 §3.3](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）。时延闸门不是新机制，
//! 就是本包的**依赖注入策略**：`assemble` / 令牌签发按档位给证，调用方拿到的
//! 令牌类型决定它能调哪些方法——**反射档超时不是被检测到，而是不可能发生**。
//!
//! ## 契约出处
//!
//! 形状逐字来自可执行论证
//! `docs/003-design/verify/latency_gate.rs`；
//! 四层预算表来自 [DE-001 §10](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。
//!
//! ## 「依赖」的边界（诚实划界）
//!
//! 令牌是 **ZST 契约类型**（零大小、零运行时开销、私有构造不可伪造）。
//! ② `actors` 的 `Reasoner` 方法签名引用 `FullModel`——这是注入策略的**类型边**，
//! 不是机制依赖：它不携带任何 ⑤ 的行为，只携带「该主体被装配在深度档」这一事实。

use async_trait::async_trait;

use crate::chat_node::ChatNode;
use crate::PromptMessage;
use crate::{CapabilityMeta, ModelFinishReason, TurnToolCallInfo};
use crate::{ModelSpend, ModelUsage};

// ── 四层时延（[DE-001 §10](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")，一等契约） ──

// 档位本身是共享语法，住 `symbio_kernel`；令牌与签发仍是本模块的部署策略。
pub use symbio_kernel::LatencyTier;

// ── 能力令牌：两档 ZST，私有构造 ⇒ 不可伪造 ─────────────────────────────

/// 反射档令牌：证明持码者处于「禁模型」档。
pub struct RuleOnly {
    _private: (),
}

/// 深度档令牌：允许完整模型调用 + 工具。
///
/// **`Copy` 不放松闸门**：签发点仍是 [`TokenIssuer`] 唯一一处（`mint` 私有），复制只是
/// 让「已经签到手的档位」能随一次派发一起被搬进 future——令牌证明的是「持码者处于哪一档」，
/// 不是「哪一个变量持有它」。
#[derive(Clone, Copy)]
pub struct FullModel {
    _private: (),
}

impl RuleOnly {
    fn mint() -> Self {
        RuleOnly { _private: () }
    }
}
impl FullModel {
    fn mint() -> Self {
        FullModel { _private: () }
    }
}

/// 令牌签发中心：**唯一**能造令牌的地方，按档位签发（一个 match，零运行时开销）。
///
/// **只有两档有令牌**。快速档曾是三档之一（`ClassifyOnly` / `issue_fast`），
/// [D-096](../../../symbio-plugin-model/README.md#d-096-快速档裁-ii-一次受限模型调用classifyonly-令牌闸门显式退役不得生成的强制力搬到返回类型与工具集判据上 "crates/symbio-plugin-model/src/README") 把它退役了：它守的端口
/// （`Classifier`）在生产里从未存在，而「快速档不得生成」这条约束在 classify 插件上
/// 由**返回类型**（`parse_verdict -> Option<Verdict>`，四变体闭集）与**工具集判据**
/// （`traverse_contributes_no_tools`）承载——换的是承重墙，不是拆掉约束。
pub struct TokenIssuer;

impl TokenIssuer {
    pub fn issue_reflex() -> RuleOnly {
        RuleOnly::mint()
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
    /// 诚实的缺口（D-044 的转写纪律同源），也绝不能被计入兜底（兜底率是
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
    /// 本轮**推理**（`reasoning_content` 累积，与 [`Self::text`] 同属这一次响应）。
    ///
    /// 为什么必须带到端口面上：判「本轮是不是答了」的那一处只看得到 `text`，而
    /// reasoning 模型常常**只有思考没有正文**就把话说完（v1 早为此立了
    /// `TurnOutput::is_reasoning_only / effective_text`）。思考只经流式桥走另一条路，
    /// 端口返回体里没有它 ⇒ 判据把「想清楚了但没复述」读成「什么也没答」⇒ 整轮
    /// 落兜底格，实测表现为界面上只剩一串连续思考、既无正文也无解释。
    ///
    /// 缺省空串：provider 不一定给（协议不给 / 最小适配器只转文本）。**空是正常的**，
    /// 但判据不得把「text 空」直接读成「无产出」——两样都空才是无产出。
    pub reasoning: String,
    /// 模型**请求**的工具调用（尚未执行）；空 = 本轮收束（落 final）。
    pub tool_calls: Vec<TurnToolCallInfo>,
    /// 这一次响应的**结束原因**（provider 原样归一化，见 [`ModelFinishReason`]）。
    ///
    /// 为什么必须带到端口面上：判「这一轮的思考算不算回答」的那一处（运行器末轮收束）
    /// 只看得到 `text` 与 `reasoning` 两段文本，而**被 `max_tokens` 掐断的半段思考
    /// 既不是答案、也不是写完的思考**。少了这一格，那条判据只能把「思考即正文」一路
    /// 成立到底：实测一次 `finish=length`、正文 0 字符的轮次，13045 字的思考草稿被
    /// 原样当答话交付，退出码还是 0。
    ///
    /// 缺省 [`ModelFinishReason::Stop`] 的含义是「自然结束」，**不属于**给不出原因的
    /// 实现——那类实现（本 trait 的默认 `generate_turn`）填 [`ModelFinishReason::Other`]
    /// 并说明是 unknown，判据据此按「原因未知」处理而不是按「已说完」处理。
    /// 带得出真实原因的适配器一律填 provider 归一化后的原值。
    pub finish: ModelFinishReason,
    /// adapter 边界实测耗时（毫秒）——与 [`LlmAdapter::generate_timed`] 同一口径。
    pub cost_ms: u64,
    /// provider **实测用量**（本轮这一次响应的）。与 [`Self::cost_ms`] 同族：
    /// 都是 adapter 边界上已经看在眼里的实测数，只是观察对象不同（耗时 vs token）。
    ///
    /// 为什么必须带出来：唯一消费方是 session 插件的 token 估算校准
    /// （`chat_loop/turn.rs` 的 `feedback_estimate`）——provider 返回的真实
    /// `output_tokens` 是校准比的分子，断在这里校准就永远停在初值 1.0，
    /// 未校准启发式的系统偏差（实测对 CJK 高估约 31%）永久无人修正，
    /// 压缩预检会把本可成功的摘要请求误判成「注定超限」。
    ///
    /// 缺省 `None`：provider 不一定给（协议或桩都不保证有），校准侧本就按
    /// 可选处理——**缺是正常的，断链才是缺陷**。
    pub usage: Option<ModelUsage>,
    /// 这一次往返的**钱**（价格 × 用量，由实现体在 adapter 边界折好）。
    ///
    /// 与 [`Self::usage`] 同族、同一次测量：`None` = 没有价格配置或没有实测用量。
    /// 唯一消费者是运行器写 `response{port=model}` 的 `spend` 键——投影只累加整数，
    /// 不在这里或读侧重算价格（见 [`crate::ModelSpend`]）。
    pub spend: Option<ModelSpend>,
}

/// 流式增量接收口（[`LlmAdapter::generate_streaming`] 的回调面）。
///
/// `Send + Sync + 'static`——适配器在自己的执行任务上逐片回调，回调体可能
/// 跨线程投递（如 [`crate::exec::ExecEventSink`] 的桥接实现）。
pub trait DeltaSink: Send + Sync + 'static {
    /// 正文增量（按序追加；适配器保证语义为「追加到正文尾部」）。
    fn on_delta(&self, text: &str);
    /// 推理增量（`reasoning_content`——模型在正文之前的思考）。
    ///
    /// 与 [`Self::on_delta`] **分属两条流**：消费端各自建节点（`msg_type` 不同），
    /// 既不合并也不互相追加。合成一条会让「模型在想什么」与「模型说了什么」在节点上
    /// 再也分不开，而这两件事在界面与事实网格里都是分开的（`ReasoningDelta`
    /// 与正文各走各的子节点）。
    ///
    /// **必填而不是给个空默认实现**：漏实现的表现是「推理静默消失」——没有编译错误、
    /// 没有告警，只是界面上永远少一块。本仓对这类「静默失效」一律要求显式表态。
    fn on_reasoning(&self, text: &str);

    /// 工具调用帧——**消息形状**的第三条出口。
    ///
    /// 两种帧都经这里（与 `stream.rs` 同构，快照 / 增量由帧自身字段区分）：
    /// - 快照（`delta = None`）：身份帧，`id` / `name` / `tool_call_id` / 参数至今
    ///   全在上面——delta 必须落在身份帧之后（前端没有渲染语义可挂）；
    /// - 窄增量（`delta = Some(片段)`）：只有 `id + delta`，接收端尾部拼接。
    ///
    /// ## 为什么是整条消息而不是拆开的参数
    ///
    /// **工具节点的构造权在分发方**（`crates/symbio-plugin-loop/src/tool_dispatch.rs`），不在这里：model 插件的
    /// 转写面已经用**同一个节点 id** 广播（流式 / 落库 / 执行三处同 id，见
    /// `tool_accumulator`），分发方稍后以同一 id 定格与落库。桥若在这里自建节点、
    /// 或把帧拆成裸参数再让消费端重组，就会出现第二张卡。所以消费端的义务只是
    /// **把帧原样送进自己的帧序**——不做类型推断，不改写身份。
    fn on_tool_frame(&self, frame: &ChatNode);
}

/// 静默接收口：会话侧执行器（`TurnRunner`，非流式形态）的委托目标——同一条
/// 执行路径，不为「不要流式」造第二条。
pub struct SilentDeltas;

impl DeltaSink for SilentDeltas {
    fn on_delta(&self, _text: &str) {}
    fn on_reasoning(&self, _text: &str) {}
    fn on_tool_frame(&self, _frame: &ChatNode) {}
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
    /// **入参是消息数组而不是一段文本**（D-048a）：
    /// 角色与正文分开，工具结果保持 `role: "tool"`——`ModelProvider::round_trip` 的签名
    /// `(system_prompt, messages: &[ChatNode], …)` 本来就是消息数组；拍成一段文本会让
    /// `role` 在 adapter 边界被拍平，模型收到的是散文而不是消息序列。
    async fn generate(
        &self,
        tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError>;

    /// 埋点版生成（SLO 校准，[DE-016 §8](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
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
    /// 请求的工具调用。
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
            // 默认实现经 `generate_streaming`——那条只转正文，推理对它不可见。
            // 空串是**如实**：这一层没有思考可带，不是把思考弄丢了。
            reasoning: String::new(),
            tool_calls: Vec::new(),
            // 同一条如实：`generate_streaming` 不带结束原因，这一层**看不见** finish。
            // 填 `Stop` 是假称「自然结束」——那正是「未知被当成完成」的形态；`Other("unknown")`
            // 说的是「这一层给不出原因」，判据据此按未知处理而不是按完成处理。
            finish: ModelFinishReason::Other("unknown".to_string()),
            cost_ms,
            usage: None,
            spend: None,
        })
    }
}

mod provider_adapter;

pub use provider_adapter::ProviderLlmAdapter;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
