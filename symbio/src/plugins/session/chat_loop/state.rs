//! 会话主循环的**状态与契约**。
//!
//! 自 `chat_loop.rs` 原样搬移（拆文件不拆行为）。含会话上下文、请求快照、
//! 单轮状态、闸门判定结果、退出原因，以及编排器与 Stop 信号。
//!
//! 可见性：`ChatOrchestrator` / `StopSignal` 是**跨模块契约**（`orchestrator.rs`、
//! `resume.rs` 经 `chat_loop::X` 引用）故为 `pub`；其余为 `pub(super)`——
//! 只在 `chat_loop` 及其子模块内可见。

use super::*;

/// MODEL 会话上下文
///
/// 设计说明：
/// - 仅包含消息列表，不包含 MODEL 请求配置
/// - system_prompt、tools、轮次上限等配置应从 model_chat::Request 获取
/// - session 用于管理会话历史（滑动窗口/自动截断/持久化）
pub(crate) struct SessionContext {
    pub messages: Vec<ChatMessage>,
    pub session: Arc<dyn ChatSession>,
}

/// 请求级不可变配置（`model_chat::Request` 的取值快照，全程只读）。
///
/// 收口前，`req.xxx.unwrap_or(cfg_defaults.yyy)` 散落在主循环各处；现在只在构造
/// 这一份快照时求值一次，下游阶段函数只读快照字段。默认值的唯一真源仍是
/// `SessionConfig`（审计 C2）——本结构只是它的**请求级投影**，不引入第二份默认值。
pub(crate) struct TurnRequest {
    /// 显式软上限（`None` = 不限制；`Some(0)` 与 `None` 同义）
    pub(crate) max_tool_rounds: Option<usize>,
    pub(crate) auto_compress: bool,
    pub(crate) enable_compact_tool: bool,
    pub(crate) tool_context_window: usize,
    pub(crate) load_history: bool,
    pub(crate) system_prompt: Option<String>,
    pub(crate) provider_id: Option<String>,
}

impl TurnRequest {
    pub(crate) fn new(req: &model_chat::Request) -> Self {
        let defaults = SessionConfig::default();
        Self {
            // 用户明确要求**不要**设置 max_tool_rounds 硬性上限（智能体会话轮次越来越
            // 多）。默认（request 未显式给出）=「无上限」；仅调用方**显式**设置时才作为
            // 软上限。`Some(0)` 与 `None` 同义（不限制）——与
            // `SessionConfig::max_tool_rounds` 的 "0 = 不限制" 契约一致，
            // 避免 0 被解释成"0 轮即熔断"。
            max_tool_rounds: req.max_tool_rounds.filter(|n| *n > 0),
            auto_compress: req.auto_compress.unwrap_or(defaults.auto_compress),
            enable_compact_tool: req
                .enable_compact_tool
                .unwrap_or(defaults.enable_compact_tool),
            tool_context_window: req
                .tool_context_window
                .unwrap_or(defaults.tool_context_window),
            // 请求级默认（该字段无配置对应项），语义为"缺省即加载历史"。
            load_history: req.load_history.unwrap_or(true),
            system_prompt: req.system_prompt.clone(),
            provider_id: req.provider_id.clone(),
        }
    }
}

/// 轮次状态：请求作用域内的可变状态。
///
/// 收口前这些量散落在 `run_chat_loop` 的循环作用域里，并以 6 个 `&mut` 参数逐个
/// 穿进 `close_turn`；现在收成一个结构体，主循环与阶段函数共享同一份状态。
#[derive(Default)]
pub(crate) struct TurnState {
    /// 用户中止标志（`provider.execute_turn` 与工具执行共享同一份）
    pub(crate) abort_flag: Arc<AtomicBool>,
    /// 已完成的工具轮次（跨轮累加；软上限判定与 fade 判定都读它）
    pub(crate) tool_rounds: usize,
    /// 长度截断自动续写次数（跨轮累加）
    pub(crate) continuation_count: u32,
    /// 水位提醒一次性标记（主动压缩成功后重置，允许上下文回落后再次提醒）
    pub(crate) nudged_this_request: bool,
    /// 增量落库锚点：本轮已落库到的下标（每轮开始时重置为本轮起始消息数）
    pub(crate) last_saved: usize,
    /// 本轮已派发、尚未产出结果的工具调用 id —— **启动条件的权威判据**。
    ///
    /// 级别 1（同步工具执行）：`close_turn` 把本批工具全部跑完才返回，回到闸门时
    /// 该集合恒为空。
    /// 级别 2（异步工具调用）：`settle_turn` 把每个工具 spawn 出去并登记 id，
    /// 完成回调逐个移除；全部移除后唤醒主循环——**不完整不唤醒**。
    pub(crate) in_flight_tools: HashSet<String>,
}

/// 主循环的唯一退出原因。
///
/// 每个出口只负责**判定原因**；收尾（提示文案 + 增量落库 + Stop 钩子 + 返回语义）
/// 统一由 [`finish_turn`] 执行——新增出口不必再记得补齐三件套。
#[derive(Debug)]
pub(crate) enum TurnExit {
    /// 正常收尾：无工具调用 / 工具待用户输入。
    Completed,
    /// 循环顶部的中止检查点：上一轮 Turn 已定稿落库，**不冒泡** `Err(Aborted)`
    /// （否则消费循环的 `persist_failure` 会把成功的 Turn 误回滚为 Failed）。
    AbortedAtBoundary,
    /// 显式软上限：先广播明确提示再退出，绝不静默。
    MaxToolRounds { max: usize },
    /// 用户中止且**在途 Turn 尚未定稿**：冒泡 `Err(Aborted)`，由消费循环收尾为
    /// Failed + 错误条 + 重试入口。
    Aborted,
    /// LLM / 编排失败：冒泡原始错误。
    Failed(PluginError),
    /// resume 已完成，无需进入主循环。
    ResumeDone,
}

/// 主循环顶部的唯一闸门：**启动条件 + 退出条件**。
#[derive(Debug)]
pub(crate) enum Gate {
    /// 条件齐备 → 进入本轮准备与推理。
    Proceed,
    /// 在途工具尚未全部产出结果 → 不唤醒本轮（详见 [`gate_turn`]）。
    WaitForTools,
    /// 必须退出，携带原因。
    Exit(TurnExit),
}

/// 本轮推理产物（`TurnOutput` 被 `into_messages` 按值消费前取出的字段）。
pub(crate) struct TurnResult {
    pub(crate) root_id: String,
    pub(crate) tools_done: Vec<ToolCallInfo>,
    pub(crate) finish: FinishReason,
    pub(crate) had_tool: bool,
}

/// Stop 钩子的幂等触发器。
///
/// 契约：**一个请求生命周期内，Stop 恰好触发一次**——无论该生命周期以何种方式
/// 结束（正常完成 / 各类错误 / abort / 软上限 / 消费循环超时 / 任务 panic）。
///
/// 实现方式：`run_chat_loop_task` 在任务最开头创建 `Arc<StopSignal>`，交给
/// [`ChatOrchestrator`]（供 `run_chat_loop` 各出口显式触发）。显式触发点携带
/// 准确的"本轮最后一条消息"；[`StopSignal::drop`] 兜底仅在显式触发全部未发生时
/// 生效（例如 chat_loop 任务 panic 被 JoinError 吞掉、消费循环 1800s 超时提前
/// return、provider 解析失败根本没能进入 loop）。Stop 的"恰好一次"由生命周期
/// 保证，而非依赖每个出口都记得调用。
///
/// 为什么显式 + RAII 双轨而非纯 RAII：`last_message` 取自 chat_loop 的
/// `context.messages`，其所有权随函数返回销毁，只有显式调用点能拿到准确值；
/// RAII 只能提供"一定会触发、但 last_message 退化为空串"的下界。兜底触发时打
/// warn 日志，使"漏调显式 fire"这类缺口在运行时可见。
pub struct StopSignal {
    parent: Option<Arc<dyn Plugin>>,
    /// Stop 钩子要投递的请求上下文（`fire_hook` 内部会再 fork 一份并设置
    /// PATH=payload，故此处持有的是原始 chat 上下文）。
    ctx: Arc<dyn InvokeRequest>,
    fired: AtomicBool,
}

impl StopSignal {
    pub fn new(parent: Option<Arc<dyn Plugin>>, ctx: Arc<dyn InvokeRequest>) -> Self {
        Self {
            parent,
            ctx,
            fired: AtomicBool::new(false),
        }
    }

    /// 本请求生命周期内 Stop 是否已触发。
    pub fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
    /// 触发 Stop；返回 `true` 表示本次调用是真正生效的那一次。
    /// `messages` 为当前请求视图消息（取末条作为 `last_message`）。
    pub async fn fire(&self, messages: &[ChatMessage]) -> bool {
        if self.fired.swap(true, Ordering::SeqCst) {
            return false;
        }
        let last_message = messages
            .last()
            .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default())
            .unwrap_or_default();
        let _ = fire_hook(
            &self.parent,
            HookEvent::Stop {
                last_message: last_message.to_string(),
            },
            self.ctx.clone(),
        )
        .await;
        true
    }
    /// 兜底触发（同步、幂等）：显式触发点一次都没执行过时，补发一次 Stop。
    ///
    /// 由 `WorkingGuard::drop`（panic / 消费循环超时 / provider 解析失败）与
    /// [`StopSignal::drop`]（最后防线）共用。Drop 语境不能 await，故投递到
    /// detached 任务；无 tokio 运行时（进程退出路径）时跳过外发并告警，
    /// `fired` 保持置位、不再重试。
    pub fn fire_fallback(&self) {
        if self.fired.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.parent.is_none() {
            // 无父插件时 fire_hook 本身就是 no-op，不打噪声日志
            return;
        }
        crate::plugin_warn!(
            "session",
            "[Stop] 显式 Stop 触发点未执行，由 StopSignal 生命周期兜底补发一次（last_message 为空）"
        );
        let parent = self.parent.clone();
        let ctx = self.ctx.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = fire_hook(
                        &parent,
                        HookEvent::Stop {
                            last_message: String::new(),
                        },
                        ctx,
                    )
                    .await;
                });
            }
            Err(_) => {
                // 没有运行时可承载：保持 fired 置位、不再重试。回滚标记没有意义——
                // 本方法的全部调用点（WorkingGuard::drop / StopSignal::drop）都处在
                // 同一条同步析构链上，下一层 Drop 同样不会有运行时，重试只会失败并
                // 重复告警（进程退出路径本就放弃了外发）。
                crate::plugin_warn!(
                    "session",
                    "[Stop] 当前无 tokio 运行时，Stop 兜底触发被跳过（进程退出路径）"
                );
            }
        }
    }
}

impl Drop for StopSignal {
    fn drop(&mut self) {
        // 最后防线：显式触发点一个都没走到（panic / 消费循环超时 / 提前 return）。
        // 已触发过则直接返回——与 `fire_fallback` 的幂等判定等价，但避免在
        // 正常路径（绝大多数请求都显式 fire 过）上多做一次原子写。
        if self.fired() {
            return;
        }
        self.fire_fallback();
    }
}

/// 会话编排器：
/// 持有唯一生效的模型服务、父插件钩子通道与预计算上下文上限（session 确定性持有）。
/// 轮次收尾状态机 `finalize_assistant_turn` 亦由本类型直接承载；
/// chat_loop 直调 `provider.execute_turn` 与 `finalize_assistant_turn`，
/// 中间不设薄委托层。
///
/// 生命周期与 [`StopSignal`] 绑定：Stop 的显式触发点在本 loop 的各出口，
/// RAII 兜底点在 `run_chat_loop_task` 的 `WorkingGuard`。
pub struct ChatOrchestrator {
    /// 唯一生效的模型服务（model 插件按上下文解析后经 CAPABILITY_VISITOR 注册；
    /// core 纯 trait 的 trait object——session 对协议实现零依赖）
    pub provider: Arc<dyn ModelProvider>,
    pub parent: Option<Arc<dyn Plugin>>,
    /// 预计算的生效上下文上限（`provider.effective_context_tokens()` 结果，
    /// 构造时由调用方传入，避免异步钩子在热路径反复触发）
    pub context_limit: u32,
    /// 本次请求生命周期的 Stop 触发器（由 `run_chat_loop_task` 创建并共享给
    /// `WorkingGuard` 兜底，见 [`StopSignal`]）
    pub stop: Arc<StopSignal>,
}

impl ChatOrchestrator {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        parent: Option<Arc<dyn Plugin>>,
        context_limit: u32,
        stop: Arc<StopSignal>,
    ) -> Self {
        Self {
            provider,
            parent,
            context_limit,
            stop,
        }
    }

    pub async fn finalize_assistant_turn(
        &self,
        root_id: &str,
        out: &TurnOutput,
        tools: &[ToolCallInfo],
        channel: &PluginChannel,
    ) {
        if out.is_reasoning_only(tools.len()) {
            // reasoning-only：模型只产生了 reasoning，没有独立的文本回复。
            //
            // 同一段 reasoning 在落库时由 build_assistant_messages 以「Text 响应子节点」承载
            // （effective_text 对「无文本回复」的回退语义）。因此这里**绝不能**再额外广播一个
            // content=reasoning 的 Text 节点——否则前端会同时持有「Reasoning 子节点」与
            // 「Text 响应子节点」两份相同内容，表现为：
            //   · 流式期间：思考块 + 一段相同文本先后出现，看起来像"同一段文本被重复写入"；
            //   · 历史刷新后：存储层本就重复（factor≈2），渲染出两份。
            //
            // 流式期间 ReasoningDelta 已经把 reasoning 累积进 reasoning_child_id 节点，
            // 此处仅将其与根 Turn 标记 Completed 即可。仅当流式期间因故未建立 Reasoning 节点时，
            // 才补发一个 Text 节点兜底（此时不存在 Reasoning 节点，不会造成重复）。
            if !out.reasoning_child_id.is_empty() {
                emit_status(
                    channel,
                    out.reasoning_child_id.clone(),
                    MessageStatus::Completed,
                )
                .await;
            } else {
                let resp_id = if out.response_text_child_id.is_empty() {
                    short_id()
                } else {
                    out.response_text_child_id.clone()
                };
                emit_update(
                    channel,
                    ChatMessage {
                        id: resp_id,
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Text),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(MessageStatus::Completed),
                        ..Default::default()
                    },
                )
                .await;
            }
            emit_status(channel, root_id.into(), MessageStatus::Completed).await;
            return;
        }

        // Mark reasoning child as completed
        if !out.reasoning.is_empty() && !out.reasoning_child_id.is_empty() {
            emit_status(
                channel,
                out.reasoning_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark response text child as completed (exists if there was text content)
        if !out.text.is_empty() && !out.response_text_child_id.is_empty() {
            emit_status(
                channel,
                out.response_text_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark tool calls (composite) as completed
        for tc in tools {
            if let Some(tc_id) = &tc.id {
                emit_status(channel, tc_id.clone(), MessageStatus::Completed).await;
            }
        }

        // Mark the root Turn node as completed
        emit_status(channel, root_id.into(), MessageStatus::Completed).await;
    }
}

#[cfg(test)]
#[path = "state.test.rs"]
mod tests;
