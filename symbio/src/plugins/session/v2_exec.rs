//! v2 执行器——`v2_mode = full` 档下，轮次经 v2 运行器**原生执行**。
//!
//! 与 [`super::v2_bridge`] 的分工（两档覆盖面互补，网格不漏轮也不双记）：
//! - bridge 档：v1 执行 + 事实**转写**进网格（收束时补记）；
//! - full 档：v2 **原生执行**——事实在生成路径上直接入格（不经转写），
//!   prompt 从转写出（同一事实源，ADR-044 同族），流式增量经 [`UiBridge`]
//!   回到 v1 的事件出口（用户看到的仍是逐字上屏的对话面）。
//!
//! 覆盖面（诚实划界）：**无工具轮与工具轮**。工具分发仍走 v1 的执行器
//! （`process_tool_calls_async`：超长结果存档、生命周期钩子、审批与恢复都在里面），
//! 由 [`super::v2_tools::SessionDispatchPort`] 经 core 的 [`crate::symbio_core::DispatchPort`]
//! 契约接入——**分发在 v2 是插件侧的实现细节，不是第二条执行链**。
//!
//! 窗口：prompt 只带最近 `context_messages` 轮（含当前轮）——事实全量
//! 入格（append-only），**视图**才是窗口。
//!
//! 兜底语义：生成失败时运行器落 fallback 事件（I3）后，本函数把失败
//! **上抛**给 chat_loop（`Failed` 出口）——用户侧的失败呈现（错误状态 +
//! 重试）与 v1 保持一致；网格里已有兜底格，重试即新一轮（N3 靠构造成立）。
//!
//! 中止语义：生成被中止时运行器**不落收束格**（网格少一格是诚实缺口，
//! ADR-045 同源），本函数上抛 `Aborted`——chat_loop 走独立出口，会话结局
//! `aborted`（**不是** `failed`）。中止与失败在**类型上**分开（`AdapterError`
//! / `TurnOutcome.aborted`），不靠错误文本猜；两者出口因此可各自演化。
//!
//! 等待用户语义：工具报 `failure_kind = pending`（confirm / ask_user）时运行器
//! 停止工具循环且**不落收束格**（本轮还没了结），本函数按正常收束返回——
//! 待用户动作由分发方广播的 `user_prompt` 节点承载（`WaitingUserAction`），
//! 与 v1 的呈现一致。恢复（用户答完续跑同一轮）是独立一批。
//!
//! 与 bridge 档的**已知差异**（中止轮 / 等待轮）：bridge 档在收束时原子转写，
//! 中止即整轮不转写（网格零增长）；full 档必须先落用户格——prompt 从网格出，
//! 用户发言不入格就会从下一轮的 prompt 里消失。同一「诚实缺口」原则的两种落地，
//! 差别源于 prompt 来源（v1 读转写 / v2 读网格）。影响面：中止轮在 full 档
//! 计入兜底率的**分母**（用户格在网格里），bridge 档不计——分子两档都不计。

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::symbio_core::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    llm_short_id, CapabilityMeta, DeltaSink, DispatchPort, Entity, Event, EventWalStore,
    ExecAbortSignal, ExecEventSink, LatencyTier, ModelProvider, Plugin, PluginError,
    ProviderLlmAdapter, Seq, Store, TokenIssuer, TurnInput, TurnOutput, TurnResume, TurnRunner,
    TurnToolCallInfo, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;

/// 恢复产生的工具交换（审批 / 问答恢复后，由 v1 恢复路径交回）。
///
/// **为什么由恢复路径交回而不是 v2 自己从会话读**：恢复方（`resume.rs`）是唯一
/// 知道「刚重跑了哪个工具、拿到什么」的地方；让 v2 去会话存储里猜「哪条是刚恢复的
/// 结果」是启发式，恢复方直给是事实。
#[derive(Debug, Clone)]
pub(crate) struct ResumedTool {
    /// 被恢复的工具名。
    pub name: String,
    /// 恢复时实际使用的参数（`approve` / `supply` 可能改写）。
    pub args: serde_json::Value,
    /// 恢复产生的结果正文。
    pub text: String,
}

/// UI 桥的帧：首片建节点（快照），后续窄追加，收束定格。语义与 model 插件的流式帧同构。
enum UiFrame {
    Snapshot {
        id: String,
        parent: String,
        text: String,
    },
    Delta {
        id: String,
        text: String,
    },
    /// 收束本轮的正文节点（工具轮的中途正文）：工具调用节点紧接着就来，
    /// 正文必须先在**同一个**节点上定格，否则它会一直转「流式中」。
    Finalize {
        id: String,
        parent: String,
        text: String,
    },
    /// 屏障：收到即回执。给「通道外的发射方」一个**同步点**——
    /// 分发方（[`super::v2_tools::SessionDispatchPort`]）既走通道（定格正文）
    /// 又直接写出口（工具节点，`process_tool_calls_async` 只认 `ExecEventSink`），
    /// 两条路不排序就会让工具卡片跑到正文之前。
    Barrier(tokio::sync::oneshot::Sender<()>),
}

/// 流式 UI 桥（DeltaSink 侧）：同步回调里只做「分帧 + 投递」——真正的
/// `emit`（异步）由执行任务里的接收端驱动（见模块文档）。
///
/// `pub(crate)`：工具分发方（[`super::v2_tools::SessionDispatchPort`]）要经
/// [`UiBridge::finalize_node`] 定格本轮正文——它必须与增量帧走**同一条通道**，
/// 否则定格帧会越过队列里未消化的增量先到，正文被追加到已定格的副本之后。
pub(crate) struct UiBridge {
    tx: mpsc::UnboundedSender<UiFrame>,
    root_id: String,
    /// 正文节点 id：首片分配、此后稳定；`finalize_node` 交还并置空（下一轮另开）。
    node: Arc<Mutex<Option<String>>>,
}

impl UiBridge {
    /// 当前正文节点 id（未定格的最后一轮正文）。
    pub(crate) fn current_node(&self) -> Option<String> {
        self.node.lock().unwrap().clone()
    }

    /// 定格本轮正文节点并**重置**（返回被定格的节点 id；本轮无正文 ⇒ `None`）。
    ///
    /// ## 为什么必须切节点
    ///
    /// 桥只在「正文首片」建节点。工具轮有多轮生成，不切就会把多轮正文粘成一条
    /// （"我先查一下""查到了"连成一句话），而且中途正文永远等不到终态。
    ///
    /// ## 为什么经通道而不是直接 `emit`
    ///
    /// 增量帧在通道里排队、由另一个任务消化。直接 `emit` 定格帧会与队列竞争：
    /// 定格（`content` 整条替换）先到、增量后到，接收端在定格正文后**继续追加**，
    /// 结果比模型说的多出一截。
    pub(crate) fn finalize_node(&self, text: &str) -> Option<String> {
        let id = self.node.lock().unwrap().take()?;
        let _ = self.tx.send(UiFrame::Finalize {
            id: id.clone(),
            parent: self.root_id.clone(),
            text: text.to_string(),
        });
        Some(id)
    }

    /// 等通道里**已排队**的帧全部落到出口（屏障）。
    ///
    /// 分发方在定格正文之后要直接写出口（工具调用节点、工具结果节点——它们由
    /// `process_tool_calls_async` 发出，那个函数只认 `ExecEventSink`）。通道里的
    /// 定格帧此刻可能还没被发射任务消化，不设同步点就会**倒序**：工具卡片先到，
    /// 正文后到，前端按到达顺序排出来的时间线是错的。
    pub(crate) async fn flush(&self) {
        let (done, wait) = tokio::sync::oneshot::channel();
        if self.tx.send(UiFrame::Barrier(done)).is_err() {
            return; // 接收端已退出（生成结束）：没有待落帧可言。
        }
        let _ = wait.await;
    }
}

impl DeltaSink for UiBridge {
    fn on_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        // 先把已有节点 id 取到局部量再判分支：`match self.node.lock()…` 的
        // 临时 guard 会存活**整个 match**（含 None 臂），在臂里再 lock 即
        // 同线程自死锁（Mutex 不可重入）。
        let existing = self.node.lock().unwrap().clone();
        let frame = match existing {
            Some(id) => UiFrame::Delta {
                id,
                text: text.to_string(),
            },
            None => {
                let id = llm_short_id();
                *self.node.lock().unwrap() = Some(id.clone());
                UiFrame::Snapshot {
                    id,
                    parent: self.root_id.clone(),
                    text: text.to_string(),
                }
            }
        };
        // unbounded 通道：发送不阻塞、不失败（接收端与生成同生命周期）。
        let _ = self.tx.send(frame);
    }
}

/// 一轮 v2 执行的入参包（与 [`TurnInput`] 同族：职责不同的多项不摊成
/// 长参数表——`too_many_arguments` 的根因是「一个函数管太多件事」，
/// 捆包让每项在调用点自带名字）。
///
/// 刻意**不持有 `ChatOrchestrator`**：那会把「执行一轮」与「编排器」绑死，
/// 而这里需要的只是它的三项（模型服务 / 插件宿主 / 本插件目录）——捆成窄接口
/// 之后，单测不必造一个完整编排器。
pub(crate) struct V2Turn<'a> {
    /// 会话（v2 WAL 目录与档位的持有者）。
    pub session: &'a PersistentChatSession,
    /// 本轮生效的模型服务（core trait 的 trait object）。
    pub provider: Arc<dyn ModelProvider>,
    /// 插件宿主（工具能力经它路由；无工具轮为 `None`）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本插件目录（工具结果存档落在这里）。
    pub session_dir: crate::symbio_core::PluginDir,
    /// 本轮请求上下文（工具路由与能力注册表都在它上面）。
    pub ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    /// 会话级系统提示词（与 v1 同一取值点——`prepare_turn_inputs`）。
    pub system_prompt: &'a str,
    /// 本轮中止信号（与 v1 同一份，中止语义不因换执行路径而丢）。
    pub abort: ExecAbortSignal,
    /// 本轮 Turn 根节点 id（UI 帧挂载的父节点）。
    pub root_id: &'a str,
    /// v1 的事件出口（UI 帧经此回到对话面）。
    pub sink: &'a ExecEventSink,
    /// 本轮用户发言（事实入格与 prompt 的同一来源）。
    pub user_text: &'a str,
    /// 对话窗口（最近轮数，含当前轮）。
    pub window_turns: u64,
    /// 本轮装配进模型的工具清单（空 = 无工具轮）。
    pub tools: &'a [CapabilityMeta],
    /// 续写锚点：`Some` ⇒ 本轮续写**已开未收束**的轮次（审批 / 问答恢复），
    /// `None` ⇒ 新开一轮。见 [`ResumedTool`] 与 core 的 `TurnResume`。
    pub resume: Option<ResumedTool>,
}

/// 一轮 v2 原生产物的**全部出口**。
pub(crate) struct V2TurnResult {
    /// 收口用产物（正文 / 节点 id）——与 v1 的 `TurnOutput` 同形，使步骤 5–7 两条路共享。
    pub output: TurnOutput,
    /// 本轮 v2 原生**额外产生**的消息（工具轮：助手中途正文 + ToolCall + 工具结果），
    /// 按产生顺序。chat_loop 把它们并入 `context.messages`——v1 的落库权威是那份
    /// 内存镜像（`persist_messages` 只写它的尾部增量），不并入就只看得见不落库。
    pub messages: Vec<ChatMessage>,
}

/// full 档的一轮执行：事实原生入格 + 生成（含工具循环）+ 流式回 UI。
pub(crate) async fn execute_turn(req: V2Turn<'_>) -> Result<V2TurnResult, PluginError> {
    let V2Turn {
        session,
        provider,
        parent,
        session_dir,
        ctx,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        window_turns,
        tools,
        resume,
    } = req;
    // 事实源：per-session v2 WAL（与桥同一个文件——两档共用一份网格）。
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("full 档需要持久会话（临时会话无事实源）".into())
    })?;
    let store = EventWalStore::open(dir.join(super::paths::V2_WAL_FILE))
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?;
    let snapshot = store.range(Seq::new(0));

    // turn 号与续写锚点：
    // - 新开轮：号 = WAL 内既有 `user.message` 计数（与桥同一口径）；
    // - 续写轮：号 = **已开未收束**的那一轮（审批 / 问答恢复），并取它的用户格 seq
    //   作溯源锚点——续写**不新开用户格**，收束仍记在该轮上（C4 按 turn 配对，
    //   另开新轮会把原轮变成永久假缺口，见 core `TurnResume`）。
    let (turn_no, resume_anchor) = match &resume {
        None => (count_user_messages(&snapshot), None),
        Some(r) => {
            let (turn, user_seq) = last_open_turn(&snapshot).ok_or_else(|| {
                PluginError::InternalError(
                    "续写轮找不到未收束的轮次（事实源与恢复路径不一致）".into(),
                )
            })?;
            (
                turn,
                Some(TurnResume {
                    user_seq,
                    call: TurnToolCallInfo {
                        id: None,
                        wire_id: None,
                        name: Some(r.name.clone()),
                        arguments: r.args.clone(),
                        parse_error: None,
                    },
                    text: r.text.clone(),
                }),
            )
        }
    };

    // 流式桥：分帧侧（同步回调）与发射侧（异步任务）经 unbounded 通道解耦。
    // 通道在桥 drop（生成结束）后关闭，接收端自然退出。
    let (tx, mut rx) = mpsc::unbounded_channel::<UiFrame>();
    let bridge = Arc::new(UiBridge {
        tx,
        root_id: root_id.to_string(),
        node: Arc::new(Mutex::new(None)),
    });
    let emit_sink = sink.clone();
    let emitter = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let message = match frame {
                UiFrame::Snapshot { id, parent, text } => ChatMessage {
                    id,
                    parent_id: Some(parent),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(text)),
                    status: Some(MessageStatus::Streaming),
                    ..Default::default()
                },
                UiFrame::Delta { id, text } => ChatMessage {
                    id,
                    delta: Some(text),
                    ..Default::default()
                },
                UiFrame::Finalize { id, parent, text } => ChatMessage {
                    id,
                    parent_id: Some(parent),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(text)),
                    status: Some(MessageStatus::Completed),
                    ..Default::default()
                },
                UiFrame::Barrier(done) => {
                    let _ = done.send(());
                    continue;
                }
            };
            emit_sink.emit(message).await;
        }
    });

    let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, abort.clone());
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: turn_no,
        text: user_text.to_string(),
        tier: LatencyTier::Deep,
        window_turns: Some(window_turns),
        resume: resume_anchor,
    };
    // 工具分发：core 只认契约（`DispatchPort`），实现是插件侧——它持插件宿主、
    // 请求上下文、转写出口与会话目录（core 认识这些即违 E-009）。
    let dispatcher = super::v2_tools::SessionDispatchPort::new(
        parent,
        ctx,
        sink.clone(),
        session_dir.dir().to_path_buf(),
        abort.clone(),
        root_id.to_string(),
        bridge.clone(),
    );
    let dispatch_ref: &dyn DispatchPort = &dispatcher;

    let outcome = TurnRunner
        .run_with_tools(
            &store,
            &llm,
            &tok,
            input,
            bridge.clone(),
            tools,
            Some(dispatch_ref),
        )
        .await
        .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;

    // 本轮的正文节点 id 必须在桥 drop 之前取走（收束帧由 chat_loop 的
    // `finalize_assistant_turn` 发出，用的就是它）。
    let child_id = bridge.current_node().unwrap_or_default();
    // 分发方产生的消息（工具轮）——先取走再释放桥，避免它随桥一起消失。
    let produced = dispatcher.take_produced();
    // 桥与分发方都归零 ⇒ 通道关闭 ⇒ 发射端排空后退出。
    drop(dispatcher);
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：运行器**未落收束格**（网格少一格是诚实缺口，ADR-045 同源），
    // 出口走 Aborted——由 chat_loop/消费循环落库为 `MessageStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`），与 v1 的中止出口同形。
    if outcome.aborted {
        return Err(PluginError::Aborted);
    }
    // 兜底已入格（I3），失败呈现交回 v1 语义（Failed 出口 + 重试）。
    if outcome.fell_back {
        return Err(PluginError::InternalError(outcome.text));
    }
    // 等待用户（`outcome.awaits_user`）：本轮尚未了结（运行器未落收束格），但
    // **按正常收束返回**——待用户动作由分发方广播的 `user_prompt` 节点承载
    // （`WaitingUserAction`），v1 的呈现（卡片 + 恢复入口）不变；恢复（用户答完
    // 续跑同一轮）是独立一批。此处的 `text` 只是模型在工具轮说过的话，不是终答。

    Ok(V2TurnResult {
        output: TurnOutput {
            text: outcome.text,
            response_text_child_id: child_id,
            ..Default::default()
        },
        messages: produced,
    })
}

/// WAL 内 `user.message` 计数（新开轮的 turn 号；与桥同一口径）。
fn count_user_messages(events: &[Event]) -> u64 {
    events
        .iter()
        .filter(|e| {
            e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE
        })
        .count() as u64
}

/// **已开未收束**的轮次（`turn × opened` 无同号收束）：返回 `(turn, 用户格 seq)`。
///
/// 与 core 的 `invariants::unresolved_turns`（C4）同一口径——等待用户动作的那一轮
/// 就是它（运行器有意不落收束格，那是「还没完」的诚实缺口）。取**最后**一个：
/// 恢复请求总是续写最近一次停顿。
fn last_open_turn(events: &[Event]) -> Option<(u64, u64)> {
    let mut open: Vec<(u64, u64)> = Vec::new();
    for e in events {
        if e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE {
            if let Some(seq) = e.seq {
                open.push((e.turn, seq.value()));
            }
        }
        if e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                EVENT_ASSISTANT_FINAL | EVENT_ASSISTANT_FALLBACK
            )
        {
            open.retain(|(turn, _)| *turn != e.turn);
        }
    }
    open.last().copied()
}

#[cfg(test)]
#[path = "v2_exec.test.rs"]
mod tests;
