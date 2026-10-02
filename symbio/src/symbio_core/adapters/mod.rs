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

/// 能做分类（快速档 / 深度档 / 自主档都满足）。
pub trait CanClassify {}
/// 能完整生成（仅深度档 / 自主档）。
pub trait CanGenerate: CanClassify {}

impl CanClassify for ClassifyOnly {}
impl CanClassify for FullModel {}
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
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::GenerationFailed(why) => write!(f, "生成失败：{why}"),
        }
    }
}

/// 流式增量接收口（[`LlmAdapter::generate_streaming`] 的回调面）。
///
/// `Send + Sync + 'static`——适配器在自己的执行任务上逐片回调，回调体可能
/// 跨线程投递（如 [`crate::symbio_core::exec::ExecEventSink`] 的桥接实现）。
pub trait DeltaSink: Send + Sync + 'static {
    /// 正文增量（按序追加；适配器保证语义为「追加到正文尾部」）。
    fn on_delta(&self, text: &str);
}

/// 静默接收口：[`crate::symbio_core::TurnRunner::run`]（非流式形态）的委托目标——同一条执行路径，
/// 不为「不要流式」造第二条。
pub struct SilentDeltas;

impl DeltaSink for SilentDeltas {
    fn on_delta(&self, _text: &str) {}
}

/// LLM 端口（⑤ 的抽象面）。**generate 只接受 `FullModel`**——反射/快速档
/// 在类型上拿不到生成能力。
///
/// 真实接线（`ModelProvider` 适配器）在 S2 e2e 落地；本端口当前由
/// [`StubLlmAdapter`]（零 LLM 桩）与后续的真实适配器实现。
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    /// 模型标识（入 final 事件的载荷，可观测）。
    fn model_id(&self) -> &str;
    /// 生成。`tok` 是闸门：没有 `FullModel` 就调不到这里。
    async fn generate(&self, tok: &FullModel, prompt: &str) -> Result<String, AdapterError>;

    /// 埋点版生成（SLO 校准，[plan/04 §1](../../../../docs/plan/04-工程落地.md)）：
    /// 除文本外返回 **adapter 边界实测耗时**（毫秒）。默认实现包一层
    /// `Instant`——桩与真实适配器零改动共享；调用方把它写到 final 事件的
    /// `cost_ms`，成本台账（③ cost_ledger）与熔断判据从此有了真实来源
    /// （G3：参数有了实测数字）。
    async fn generate_timed(
        &self,
        tok: &FullModel,
        prompt: &str,
    ) -> Result<(String, u64), AdapterError> {
        let start = std::time::Instant::now();
        let out = self.generate(tok, prompt).await?;
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
        prompt: &str,
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let out = self.generate_timed(tok, prompt).await?;
        sink.on_delta(&out.0);
        Ok(out)
    }
}

/// 零 LLM 桩——S2 彩排与测试用。可注入**确定性失败**，用于演练兜底路径。
///
/// 真实适配器（包 `ModelProvider`）由 e2e 的 MockLlm / 真实 provider 接线承接。
pub struct StubLlmAdapter {
    model: &'static str,
    /// 非空 ⇒ generate 一律返回该错误（演练兜底）。
    fail_with: Option<&'static str>,
    /// 注入的延迟毫秒（演练 cost_ms 埋点；测试配合 `tokio::time` 使用）。
    delay_ms: u64,
    /// 非空 ⇒ 逐片生成（演练流式回调；全文 = 各片拼接，与单发等价）。
    chunks: Option<Vec<String>>,
}

impl StubLlmAdapter {
    pub fn succeed(model: &'static str) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms: 0,
            chunks: None,
        }
    }

    /// 逐片生成的桩：演练流式回调（`generate_streaming` 覆写逐片送出）。
    pub fn succeed_streaming(model: &'static str, chunks: &[&str]) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms: 0,
            chunks: Some(chunks.iter().map(|c| c.to_string()).collect()),
        }
    }

    /// 一律失败的桩：演练「生成失败 → 兜底必须产生事件」。
    pub fn always_fail(message: &'static str) -> Self {
        StubLlmAdapter {
            model: "stub",
            fail_with: Some(message),
            delay_ms: 0,
            chunks: None,
        }
    }

    /// 带注入延迟的桩（演练时延埋点）。
    pub fn with_delay(model: &'static str, delay_ms: u64) -> Self {
        StubLlmAdapter {
            model,
            fail_with: None,
            delay_ms,
            chunks: None,
        }
    }
}

#[async_trait]
impl LlmAdapter for StubLlmAdapter {
    fn model_id(&self) -> &str {
        self.model
    }

    async fn generate(&self, _tok: &FullModel, prompt: &str) -> Result<String, AdapterError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        match self.fail_with {
            Some(msg) => Err(AdapterError::GenerationFailed(msg.to_string())),
            None => match &self.chunks {
                // 分片桩：全文 = 各片拼接（与单发等价——流式只是帧的形态）。
                Some(chunks) => Ok(chunks.concat()),
                None => Ok(format!("[{}] {}", self.model, prompt)),
            },
        }
    }

    async fn generate_streaming(
        &self,
        tok: &FullModel,
        prompt: &str,
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let Some(chunks) = &self.chunks else {
            // 无分片配置 ⇒ 走默认降级（一次性全文）。
            return LlmAdapter::generate_streaming(&DelegateGen(self), tok, prompt, sink).await;
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
struct DelegateGen<'a>(&'a StubLlmAdapter);

#[async_trait]
impl LlmAdapter for DelegateGen<'_> {
    fn model_id(&self) -> &str {
        self.0.model_id()
    }
    async fn generate(&self, tok: &FullModel, prompt: &str) -> Result<String, AdapterError> {
        self.0.generate(tok, prompt).await
    }
}

mod provider_adapter;

pub use provider_adapter::ProviderLlmAdapter;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
