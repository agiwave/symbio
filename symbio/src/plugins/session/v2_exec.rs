//! v2 执行器——`v2_mode = full` 档下，**无工具挂载的轮次**经 v2 运行器执行。
//!
//! 与 [`super::v2_bridge`] 的分工（两档覆盖面互补，网格不漏轮也不双记）：
//! - bridge 档：v1 执行 + 事实**转写**进网格（收束时补记）；
//! - full 档：v2 **原生执行**——事实在生成路径上直接入格（不经转写），
//!   prompt 从转写出（同一事实源，ADR-044 同族），流式增量经 [`UiBridge`]
//!   回到 v1 的事件出口（用户看到的仍是逐字上屏的对话面）。
//!
//! 覆盖面（诚实划界）：**无工具挂载的轮次**。有工具挂载的轮次回退 v1
//! （工具分发/审批/恢复都在 v1 侧，工具轮 v2 化是独立一批），事实照常
//! 经桥转写——由 `finish_turn` 的门控保证（`TurnState::v2_executed`）。
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
//! 与 bridge 档的**已知差异**（中止轮）：bridge 档在收束时原子转写，中止即
//! 整轮不转写（网格零增长）；full 档必须先落用户格——prompt 从网格出，用户
//! 发言不入格就会从下一轮的 prompt 里消失。同一「诚实缺口」原则的两种落地，
//! 差别源于 prompt 来源（v1 读转写 / v2 读网格）。影响面：中止轮在 full 档
//! 计入兜底率的**分母**（用户格在网格里），bridge 档不计——分子两档都不计。

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    llm_short_id, DeltaSink, Entity, EventWalStore, ExecAbortSignal, ExecEventSink, LatencyTier,
    ModelProvider, PluginError, ProviderLlmAdapter, Seq, Store, TokenIssuer, TurnInput, TurnOutput,
    TurnRunner, Verb, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;

/// UI 桥的帧：首片建节点（快照），后续窄追加。语义与 model 插件的流式帧同构。
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
}

/// 流式 UI 桥（DeltaSink 侧）：同步回调里只做「分帧 + 投递」——真正的
/// `emit`（异步）由执行任务里的接收端驱动（见模块文档）。
struct UiBridge {
    tx: mpsc::UnboundedSender<UiFrame>,
    root_id: String,
    /// 正文节点 id：首片分配、此后稳定；收口按它落终态帧。
    node: Arc<Mutex<Option<String>>>,
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
pub(crate) struct ToolFreeTurn<'a> {
    /// 会话（v2 WAL 目录与档位的持有者）。
    pub session: &'a PersistentChatSession,
    /// 本轮生效的模型服务（core trait 的 trait object）。
    pub provider: Arc<dyn ModelProvider>,
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
}

/// full 档的无工具轮执行：事实原生入格 + 生成 + 流式回 UI。
///
/// 返回的 [`TurnOutput`] 只填 chat_loop 收口需要的字段（text / 节点 id）——
/// 工具与推理产物在无工具轮上恒为空，这是事实不是省略。
pub(crate) async fn execute_tool_free_turn(
    req: ToolFreeTurn<'_>,
) -> Result<TurnOutput, PluginError> {
    let ToolFreeTurn {
        session,
        provider,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        window_turns,
    } = req;
    // 事实源：per-session v2 WAL（与桥同一个文件——两档共用一份网格）。
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("full 档需要持久会话（临时会话无事实源）".into())
    })?;
    let store = EventWalStore::open(dir.join("v2-events.wal"))
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?;

    // turn 号 = WAL 内既有 user.message 计数（与桥同一口径）。
    let turn_no = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| {
            e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE
        })
        .count() as u64;

    // 流式桥：分帧侧（同步回调）与发射侧（异步任务）经 unbounded 通道解耦。
    // 通道在桥 drop（生成结束）后关闭，接收端自然退出。
    let (tx, mut rx) = mpsc::unbounded_channel::<UiFrame>();
    let node: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let bridge = UiBridge {
        tx,
        root_id: root_id.to_string(),
        node: node.clone(),
    };
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
            };
            emit_sink.emit(message).await;
        }
    });

    let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, abort);
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: turn_no,
        text: user_text.to_string(),
        tier: LatencyTier::Deep,
        window_turns: Some(window_turns),
    };
    let outcome = TurnRunner
        .run_streaming(&store, &llm, &tok, input, Arc::new(bridge))
        .await
        .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;
    // 桥已随 Arc 归零 drop ⇒ 通道关闭 ⇒ 发射端排空后退出。
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

    let child_id = node.lock().unwrap().clone().unwrap_or_default();
    Ok(TurnOutput {
        text: outcome.text,
        response_text_child_id: child_id,
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "v2_exec.test.rs"]
mod tests;
