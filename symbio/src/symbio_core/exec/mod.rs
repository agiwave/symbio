//! 执行期统一原语：**事件出口（出）** 与 **中止信号（入）**。
//!
//! ## 为什么需要这一层
//!
//! 历史上，执行期（LLM 单轮 / 工具调用）把「出」与「入」两个方向都压在同一个
//! [`crate::symbio_core::PluginChannel`] 上：节点事件先 `serde_json::to_value` 装箱成
//! [`crate::symbio_core::PluginFrame::Data`] 发出，中止信号反向以帧回灌；消费循环收到
//! 后再 `serde_json::from_value` 解回来只为分辨「这是 `Warn` 还是别的 op」。
//!
//! 问题不在帧本身，而在**帧被用错了地方**：`PluginChannel` 是**跨进程传输原语**
//! （前端链路要过 Tauri IPC / HTTP），而 `provider.execute_turn` 与 `tool.execute`
//! 的两端**始终在同一个进程、同一个 spawn 里**（见 `plugins/model/bound_provider.rs`）。
//! 进程内的调用不该付进程外的代价。
//!
//! ## 拆成两个语义单一的原语
//!
//! - [`ExecEventSink`]（出）：生产者只做两件事——`emit(ChatMessage)`（消息图变更，
//!   帧**就是一条消息**：`delta` 追加 / `content` 整条替换 / `status` 状态迁移）
//!   与 `warn(Option<String>)`（会话级告警，落在会话节点状态，不是消息帧）。
//!   去哪由实现决定：进程内直连**转写唯一写入点**（零 serde），内部请求走
//!   [`ExecEventSink::Null`]。
//! - [`ExecAbortSignal`]（入）：把历史上三条并存的中止感知路径（显式 Abort 帧 /
//!   共享 `abort_flag` 轮询 / 通道取消）**收敛成一条**，且不再需要轮询——
//!   `abort()` 置位的同时唤醒所有等待者。
//!
//! 于是 `PluginChannel` 退回它本来的定位：**跨进程传输**（前端经
//! `event_bus/subscribe` 收帧，走 `PluginPayload::Session`），不再承担执行期协议。
//!
//! ## 不变量（改动时不得破坏）
//!
//! - 转写仍然只有**一个写入点**（[`ExecTranscriptWriter::apply`] 的实现体是
//!   `Transcript::apply`），事件只是换了条路抵达它；
//! - 告警仍是**会话级状态**（VDFS watch 域），走出口的 [`ExecEventSink::warn`]
//!   通道分派到会话节点、不进转写——消息帧与会话状态两个域不混流；
//! - 中止只有一个置位入口 [`ExecAbortSignal::abort`]，读侧只有 [`ExecAbortSignal::is_aborted`]
//!   与 [`ExecAbortSignal::cancelled`]——不再有第二条「标志位之外的中止来源」。

use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt};
use crate::symbio_core::{ABORT_SIGNAL, EVENT_SINK};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 转写唯一写入点的抽象。
///
/// `symbio_core` 不认识 `Transcript`（它在 session 插件里），因此只约定这个最小面；
/// 生产实现是 session 插件的 `TranscriptSink`——`apply` 内部调用
/// `Transcript::apply`，`warn` 路由到会话状态出口。
#[async_trait]
pub trait ExecTranscriptWriter: Send + Sync + 'static {
    /// 消费一条消息帧。实现体必须保证：同一会话内按调用顺序生效。
    async fn apply(&self, message: ChatMessage);

    /// 会话级告警（可恢复）：`Some(text)` 设置 / `None` 清除。
    ///
    /// 告警是**会话节点状态**（VDFS watch 域），不是消息流上的帧——它与消息
    /// 分属两个域，所以不在 [`ExecTranscriptWriter::apply`] 的载荷里。默认实现忽略：
    /// 不感知会话状态的出口（测试 / 转播桥）没有可落的去处。
    async fn warn(&self, _warning: Option<String>) {}
}

/// 出口的**进度观测点**：与出口共享的发射计数（单调不减）。
///
/// ## 存在的唯一理由
///
/// 执行层必须能区分「工具在推进」与「工具挂死」。收口前这个判断只能靠时间，
/// 于是并存两个魔法数：非流式工具的总时长上限（600s）与流式工具的空闲上限
/// （180s）——同一个问题两套口径，且**总时长上限会误杀长任务**：一个跑十几分钟
/// 但一直在发事件的子智能体（`agent_run`）与一个真的挂死的工具，在「总时长」
/// 这一维上完全同形。
///
/// 有了计数，「有进展就不算挂死」成为**可判定**的规则：执行层只需比较两次读数。
/// 于是总时长上限被取消，只留一个空闲上限，且不再需要为长任务开特例。
///
/// 计数挂在出口上而不是别处，是因为出口**本来就是**「工具还活着」的唯一证据源：
/// 工具唯一的出方向动作就是 [`ExecEventSink::emit`]。
#[derive(Clone, Default, Debug)]
pub struct ExecEventSinkProgress(Arc<AtomicU64>);

impl ExecEventSinkProgress {
    /// 已发生的发射次数。
    pub fn emitted(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// 执行期**事件出口**（唯一出口，出方向）。
///
/// 生产者（`ModelProvider::execute_turn` / 工具执行）只调 [`Self::emit`]，
/// 不感知事件最终去哪：
/// - [`ExecEventSink::Direct`]：进程内直连转写唯一写入点（零 serde 往返）；
/// - [`ExecEventSink::Null`]：静默——内部请求（上下文压缩）刻意不产生任何可见帧。
///
/// 跨进程（前端实时面）不走本类型：那是 `PluginPayload::Session` 的职责，
/// 两者是**不同的面**，不要合并。
#[derive(Clone)]
pub enum ExecEventSink {
    /// 进程内直连：直接进转写唯一写入点；附带进度观测点（见 [`ExecEventSinkProgress`]）。
    Direct(Arc<dyn ExecTranscriptWriter>, ExecEventSinkProgress),
    /// 静默出口：`emit` 是 no-op。
    ///
    /// 用途唯一且明确——**内部请求**（上下文压缩摘要）不得产生任何可见帧：
    /// 压缩发生在 Turn 创建之前，若走真实出口会在前端留下永远「正在思考…」的
    /// 空 Turn 骨架（每轮压缩尝试累积一个）。历史上这一需求靠「哑通道 + 换 rx」
    /// 的 hack 实现（`compress.rs` 里的 `std::mem::replace(&mut channel.rx, dummy_rx)`），
    /// 现在是一个显式的出口选择。
    Null,
}

impl ExecEventSink {
    /// 构造进程内直连出口。
    pub fn direct(writer: Arc<dyn ExecTranscriptWriter>) -> Self {
        Self::Direct(writer, ExecEventSinkProgress::default())
    }

    /// 静默出口（内部请求专用）。
    pub fn silent() -> Self {
        Self::Null
    }

    /// 从请求上下文取出口；**缺席 ⇒ 静默**。
    ///
    /// 「有没有出口」由 `ctx` 承载（键 [`EVENT_SINK`]），不由调用形态决定：
    /// 会话编排层在发起工具调用前写入，工具侧只读。于是同一个能力
    /// 被 `route()` 直接调用时自然静默，**不需要为它造第二条代码路径**。
    pub fn of(ctx: &dyn PluginInvokeRequest) -> Self {
        ctx.get(EVENT_SINK).unwrap_or(ExecEventSink::Null)
    }

    /// 取本出口的进度观测点（与工具持有的那份共享同一计数）。
    ///
    /// 静默出口没有进展可言：返回一个恒为 0 的观测点——语义正确（`Null` 出口
    /// 从不发射），且调用方不必分情形。
    pub fn progress(&self) -> ExecEventSinkProgress {
        match self {
            Self::Direct(_, progress) => progress.clone(),
            Self::Null => ExecEventSinkProgress::default(),
        }
    }

    /// 送出一条消息帧。
    ///
    /// 这是执行期**唯一**的出方向动作：节点出现 / 内容增长（`delta`）/
    /// 整条替换（`content`）/ 状态迁移（含 `removed` 即删除）都只是一条
    /// [`ChatMessage`] 的不同字段，帧面语义由字段本身给出，接收端不做类型推断。
    pub async fn emit(&self, message: ChatMessage) {
        match self {
            ExecEventSink::Direct(writer, progress) => {
                progress.bump();
                writer.apply(message).await
            }
            ExecEventSink::Null => {}
        }
    }

    /// 会话级告警：**不是** `emit(…)` —— 告警是会话节点状态，
    /// 与消息分属两个域。生产实现（`TranscriptSink`）路由到
    /// `emit_session_state(Warning)`；静默出口忽略。
    pub async fn warn(&self, warning: Option<String>) {
        match self {
            ExecEventSink::Direct(writer, _) => writer.warn(warning).await,
            ExecEventSink::Null => {}
        }
    }
}

impl std::fmt::Debug for ExecEventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecEventSink::Direct(..) => f.write_str("ExecEventSink::Direct"),
            ExecEventSink::Null => f.write_str("ExecEventSink::Null"),
        }
    }
}

/// 执行期**环境**：一次「执行」的出/入两个方向的唯一来源。
///
/// ## 为什么要有这个具名类型
///
/// 收口前，执行期输入散在**两处、三种形态**：
/// - `ModelProvider::execute_turn` 写成**显式参数**（`sink` / `abort`）；
/// - `Capability::execute` 塞进**请求信封**（`ctx`）——出口、中止、参数、会话上下文
///   （`WORKDIR` / `MODE` / `RISK_LEVEL`…）与路由键（`PATH` / `PARENT` / `trace_id`）
///   混在同一个键值袋里，于是每个工具都得自己记住「我该读哪些键」。
///
/// 但两者其实是**同一件事**：一次带中止的流式执行——出方向写事件、入方向读中止、
/// 最后给一个终局值。把这件事具名化之后，两个接口同形：
///
/// ```text
/// Capability::execute(args, env, ctx)   -> Result<Value, PluginError>
/// ModelProvider::execute_turn(inputs, env) -> Result<TurnOutput, PluginError>
/// ```
///
/// 差别只剩 `ctx`：**工具是被路由、被注册的**（要转发、要解析挂载），因此还需要
/// 信封；模型执行不被路由，也就没有信封。这是真实差异，不强行抹平。
///
/// ## 与请求信封（`ctx`）的分工
///
/// - `ctx` = 「这次调用**从哪条路径来**」——路由、父子插件、trace、能力注册表；
/// - `ExecEnv` = 「这次调用**要怎么跑**」——出口、中止。
///
/// 信封仍然在，只是不再挡在每个工具面前：出口与中止从信封里的两个**无名键**
/// 升级为**具名类型**，缺席（`route()` 直连调用）自动降级为静默 / 永不中止。
#[derive(Clone)]
pub struct ExecEnv {
    sink: ExecEventSink,
    abort: ExecAbortSignal,
}

impl ExecEnv {
    pub fn new(sink: ExecEventSink, abort: ExecAbortSignal) -> Self {
        Self { sink, abort }
    }

    /// 从请求信封装配环境（工具侧）。**这是执行期唯一「拆信封」的地方**——
    /// 工具不再自己 `ctx.get(EVENT_SINK)` / `ctx.get(ABORT_SIGNAL)`。
    pub fn from_request(ctx: &dyn PluginInvokeRequest) -> Self {
        Self {
            sink: ExecEventSink::of(ctx),
            abort: ExecAbortSignal::of(ctx),
        }
    }

    /// 出方向：事件出口。
    pub fn sink(&self) -> &ExecEventSink {
        &self.sink
    }

    /// 入方向：中止信号。
    pub fn abort(&self) -> &ExecAbortSignal {
        &self.abort
    }
}

impl std::fmt::Debug for ExecEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecEnv")
            .field("sink", &self.sink)
            .field("abort", &self.abort)
            .finish()
    }
}

/// 执行期**中止信号**（唯一入方向原语）。
///
/// ## 为什么是一个值类型而不是三条路径
///
/// 收口前，中止感知有三条并存路径，各自覆盖不同的调用形态：
/// 1. **显式 Abort 帧**：消费循环把 `ControlSignal::Abort` 投进执行期通道；
/// 2. **共享标志位轮询**：`Arc<AtomicBool>` 被外部置位——历史上这是**内部请求
///    （上下文压缩）唯一的中止感知路径**（压缩请求挂在静默哑通道上，rx 永无帧，
///    只有共享标志位会变）；
/// 3. **通道取消**：`cancel_token` 被强制取消（消费循环超时兜底 / 会话销毁）。
///
/// 三条路径的存在本身就是「一个通道承担两种职责」的产物。现在合并为一条：
/// [`Self::abort`] 同时置位与唤醒，等待方只需 [`Self::cancelled`]，**无需轮询**
/// （历史上 `wait_for_abort_signal` 每 100ms 醒一次，纯属为路径 2 让路）。
///
/// ## 生命周期语义
///
/// `ExecAbortSignal` 是 `Clone` 的（内部 `Arc`）：发起方（消费循环）与执行方
/// （`run_chat_loop`）各持一份。消费循环在退出时调用 [`Self::abort`]，
/// 等价于历史上「通道关闭 ⇒ 执行方中止」——只是从隐式（drop 掉 sender）
/// 变成显式（一次命名调用）。
#[derive(Clone, Default)]
pub struct ExecAbortSignal {
    flag: Arc<AtomicBool>,
    cancel: CancellationToken,
}

impl ExecAbortSignal {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            cancel: CancellationToken::new(),
        }
    }

    /// 从请求上下文取中止信号；**缺席 ⇒ 一个永不中止的独立信号**。
    ///
    /// 缺席不是错误：`route()` 直接调用没有编排层，也就没有中止来源。
    /// 给一个"永不触发"的信号比让调用方到处写 `if let Some` 诚实——
    /// 等待方 `cancelled()` 会永远 pending，语义与"没人会中止我"一致。
    pub fn of(ctx: &dyn PluginInvokeRequest) -> Self {
        ctx.get(ABORT_SIGNAL).unwrap_or_default()
    }

    /// 是否已中止。读侧的唯一判定。
    pub fn is_aborted(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 置位并立即唤醒所有等待者。**唯一的置位入口**。
    pub fn abort(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.cancel.cancel();
    }

    /// 等到中止（已中止则立即返回）。
    ///
    /// 取代历史上 `wait_for_abort_signal` 的 `select!{ flag 轮询 | cancel | rx }`
    /// 三臂——`abort()` 已经承担了「唤醒」职责，因此不再需要轮询臂。
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    /// 共享标志位（只读用途：传给仍需 `Arc<AtomicBool>` 的既有代码，如
    /// 工具执行器的 `is_aborted` 观测点）。**不得**用它绕过 [`Self::abort`] 置位。
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
}

impl std::fmt::Debug for ExecAbortSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExecAbortSignal({})", self.is_aborted())
    }
}

#[cfg(test)]
mod tests;
