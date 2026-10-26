//! 测试替身：core 的测试面**不再出根**（[DE-023](https://example.invalid) A1 之后的债）。
//!
//! ## 为什么它们住这里而不是 core
//!
//! 拆 crate（D-191）之后，`#[cfg(test)]` 在 core 里**只在编译 core 自己的测试时生效**，
//! 别的 crate 依赖它时那个分支根本不存在——所以测试替身放 core 就只能常开为 `pub`，
//! 把 C-004 账本撑高一截（那是虚高，不是共享面）。
//!
//! 落点是这里：`symbio-testkit` 是**普通 crate**，插件以 `[dev-dependencies]` 引入，
//! 生产构建根本看不见它。测试替身归测试设施，core 的公开面因此只剩「真被两处以上
//! 生产代码看见」的东西。
//!
//! ## 消费方
//!
//! | 替身 | 消费方 |
//! |---|---|
//! | [`StubLlmAdapter`] | `symbio-plugin-loop` 的 `turn_runner.test.rs` |
//! | [`trivial_actor_spec`] | `symbio-plugin-loop`（`task_events` / `turn_runner`）· `symbio-plugin-session`（`store/wal`） |

use async_trait::async_trait;
use symbio_core::{
    production_matrix, ActorSpec, AdapterError, CapabilityMeta, DeltaSink, FullModel, LlmAdapter,
    LlmTurn, ModelFinishReason, Pattern, PromptMessage, Scope,
};

/// 零 LLM 桩——彩排与测试用。可注入**确定性失败**，用于演练兜底路径。
///
/// 真实适配器（包 `ModelProvider`）由 e2e 的 MockLlm / 真实 provider 接线承接。
/// 桩只有测试这一个消费面，故整组（结构 + 构造 + 实现 + 分片委托）进
/// `#[cfg(test)]`——不占生产面，也就没有「接入生产」这一步可走。
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

#[async_trait]
impl LlmAdapter for StubLlmAdapter {
    fn model_id(&self) -> &str {
        self.model
    }

    /// 桩的 `cost_ms` 取**注入延迟**，不取墙钟。
    ///
    /// 默认实现用 `Instant` 实测；零延迟桩在并发测试里偶尔会是 1ms，于是
    /// 「重放台账 == 直写台账」这类判据会在同一份代码上随机红（门禁
    /// 实际撞到过）。桩的职责是给测试一个**可复现**的外部源，不是复刻调度
    /// 抖动：延迟参数就是它声明的耗时，0 就是 0。
    async fn generate_turn(
        &self,
        _tok: &FullModel,
        messages: &[PromptMessage],
        _tools: &[CapabilityMeta],
        sink: std::sync::Arc<dyn DeltaSink>,
    ) -> Result<LlmTurn, AdapterError> {
        if self.abort {
            return Err(AdapterError::Aborted);
        }
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if let Some(msg) = self.fail_with {
            return Err(AdapterError::GenerationFailed(msg.to_string()));
        }
        let text = match &self.chunks {
            Some(chunks) => {
                for c in chunks {
                    sink.on_delta(c);
                }
                chunks.concat()
            }
            None => {
                let text = format!(
                    "[{}] {}",
                    self.model,
                    messages.last().map(|m| m.text.as_str()).unwrap_or("")
                );
                sink.on_delta(&text);
                text
            }
        };
        Ok(LlmTurn {
            text,
            reasoning: String::new(),
            tool_calls: Vec::new(),
            finish: ModelFinishReason::Other("unknown".to_string()),
            cost_ms: self.delay_ms,
            usage: None,
            spend: None,
        })
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
struct DelegateGen<'a>(&'a StubLlmAdapter);

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

/// S01 的平凡值主体（[DE-031 §4](../../../docs/003-design/031-最小闭环.md "DE-031-最小闭环")）：
/// 全规则驱动、预算放宽、单主体——平凡值下系统完整运行。
///
/// **测试面**构造点（ADR-067：生产按真实授权行立规格）。它住测试设施而不是
/// `symbio_core`：`cfg(test)` 对下游 crate 不生效（见本模块文档），放 core 就只能
/// 以 `pub` 常开，那是把测试面的开销记到公开面账上。
///
/// ## `capabilities` 取该主体**自己的授权行**
///
/// 空表与「平凡值下系统**完整运行**」自相矛盾：收束闸判读规格身上的能力集之后，
/// 空表意味着平凡值连一轮对话都收束不了。所以它必须是**真值**，不能是「算了却
/// 看不见」的占位。
///
/// 平凡值 = **权限取实（该主体的登记行）、模式与预算取平凡**（规则驱动、
/// 放宽预算、根作用域）。`session/main` 因此持 `reply.*` 能收束，而
/// `session/autonomous`（只持 `define.work`）照旧无发言权——「自主行为不得冒充
/// 用户对话」这条不因平凡值而失效。
pub fn trivial_actor_spec(principal: impl Into<String>) -> ActorSpec {
    let principal = principal.into();
    ActorSpec {
        capabilities: production_matrix().grant_names_of(&principal),
        pattern: Pattern::Decider,
        budget_ms: 60_000,
        scope: Scope::Root,
        principal,
    }
}

#[cfg(test)]
#[path = "stubs.test.rs"]
mod tests;
