//! v2 执行器——轮次经 v2 运行器**原生执行**（唯一执行路径，D-051）。
//!
//! 事实与呈现同源（网格不漏轮也不双记）：事实在生成路径上**直接入格**——轮次事实
//! （`u-{turn}` 开口 / 收束格）由 v2 运行器落，派生事实由本模块轮末补（见下）；
//! prompt 从转写出（同一事实源，D-044 同族），流式增量经 [`UiBridge`]
//! 回到事件出口（用户看到的仍是逐字上屏的对话面）。
//!
//! 覆盖面（诚实划界）：**无工具轮与工具轮**。工具分发走工具执行层
//! （`process_tool_calls_async`：超长结果存档、生命周期钩子、审批与恢复都在里面），
//! 由 [`super::tool_dispatch::SessionDispatchPort`] 经插件本地的
//! [`super::tool_dispatch::ToolDispatch`] 三动作契约接入——**分发是插件侧的实现细节，
//! 不是第二条执行链**。
//!
//! ## 反射档（S11 快路）：唯一不调模型的一条
//!
//! 开 `skill_fast_path` 且本轮发言**逐字命中**一条已编译技能时，本轮改走快路——
//! 以技能正文收束，**一次模型调用都不发生**。**选中**发生在环上
//! （`policy.reflex` 行，出口格 `turn.stage{stage:"reflex"}`）；本模块只消费那次
//! 选中的结果：正文由 `policy.reflex` 的执行体经 [`TurnSession::set_reflex`] 寄存，
//! 由 `close-turn` 的执行体落成同一格 `chat.assistant.final`（D-163）。
//! 它不是"又一条执行链"：产物落的是**同一轮的三格**（`u-{turn}` / `turn.dispatch` /
//! `f-{turn}`），收束后仍走同一段 `record_learning`，收口（步骤 5–7）也完全共享；
//! 变的只是**生成那一格**由谁产出（快路这一轮没有 `llm` 形参 ⇒ "反射档调模型"写不出来）。
//!
//! 两条边界，缺一条都会静默走错：
//! - **只对新开轮**：续写轮不新开用户格（调用方直给原轮的 `user_seq`），而反射档的
//!   落格形状就是新开一轮 ⇒ 走它会撞幂等键（`Duplicate`），既完不成收束也把本轮变成错误；
//! - **开关默认关**：关着时候选集恒空，本轮照常走 [`drive`]。
//!
//! 反射档把 `actor.budget_ms` 按档位派生（`LatencyTier::Reflex.budget_ms()`），
//! 该值随主体走、并由接缝以**数值**写入 `turn.dispatch` 载荷的 `budget_ms`
//! （/ ）——`invariants::declared_budget_ms` 优先读它，`ActorSpec.budget_ms`
//! 由它获得真读方。缺该数值的历史事件回退读 `tier`
//! 字符串再映射，两条路径读的是**同一条事实的两种形态**，见 `declared_budget_ms` 文档。
//!
//! ## 收束派生事实（本模块已覆盖）
//!
//! 轮次事实（`u-{turn}` 开口 / 收束格）由运行器原生入格；**派生事实不是轮次事实**，
//! 而是本轮的派生副作用——运行器一处都不写，写方只有轮末这两个函数
//! （溯源锚 = 这里原生写的 `u-{turn}` 格）：
//!
//! - [`super::derived_facts::record_learning`]：步 11 编码 / 步 12 检索锚 / 步 13 巩固 /
//!   步 22 技能观测与编译；
//! - [`super::derived_facts::record_derived`]：**效果**（`ToolBehavior::after` 交出的
//!   草稿：承诺 / 任务表 / …，DE-025 D-207）与熔断（`CircuitBreaker::break_event`，
//!   S8 步 20）——前者的数据来源是工具执行层交回的 `EffectInput`，后者是熔断理由，
//!   都由 [`super::tool_dispatch::SessionDispatchPort`] 经 `take_derived` 交回。
//!
//! 效果**不按工具名分派**：本模块只补宿主面（轮锚 / 溯源 / 主体）再问声明，具体是哪
//! 一笔账由能力包自己实现。加一个效果不改这里一行。
//!
//! ## 写侧授权闸（判在运行器）
//!
//! **写侧授权闸**（[plan/01 §7](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）判的是「主体此刻
//! 能否写本轮的收束格」（`reply.first` / `reply.append`），由运行器在落收束格**之前**
//! 判（`turn_runner::closure_gate`，谓词 `can_reply` 只定义一次）。拒绝 ⇒ 收束格
//! 不落、该轮留在未收束态（`check_all` 的 C4 报得出），本模块据 `closure_denied`
//! 跳过本轮的派生事实（见下方收束派生事实的收尾）。
//!
//! 窗口：prompt 只带最近 `context_messages` 轮（含当前轮）——事实全量
//! 入格（append-only），**视图**才是窗口。
//!
//! 兜底语义：生成失败时运行器落 fallback 事件（I3）后，本函数把失败
//! **上抛**给 chat_loop（`Failed` 出口）——用户侧的失败呈现（错误状态 +
//! 重试）走那条出口；网格里已有兜底格，重试即新一轮（N3 靠构造成立）。
//!
//! 中止语义：生成被中止时运行器**照落收束格**（`f-{turn}`，kind 仍是
//! `chat.assistant.final`——中止是事实的**状态**，不是另一种事实；载荷
//! `status: "aborted"` + `terminated: { at }` + 顶层 `reason: interrupted`），本函数上抛
//! [`TurnExecExit::Aborted`]——chat_loop 走独立出口，会话结局 `aborted`
//! （**不是** `failed`），半截产出随出口入格。
//! 中止与失败在**类型上**分开（`AdapterError` / `TurnOutcome.aborted`），
//! 不靠错误文本猜；两者出口因此可各自演化。
//!
//! 等待用户语义：工具报 `failure_kind = pending`（confirm / ask_user）时运行器
//! 停止工具循环，落一格**进展格**（`chat.assistant.progress`，`turn × progressed`，
//! 载荷 `status: "pending"`）——本轮还没了结，收束格留给恢复幕（一轮至多一条）。
//! 本函数按正常收束返回——待用户动作由分发方广播的 `user_prompt` 节点承载
//! （`WaitingUserAction`），呈现为卡片 + 恢复入口。恢复（用户答完续跑同一轮）走 [`TurnResume`]，**不重开用户格**。
//!
//! 但**派生事实两幕都写**（等待幕与恢复幕都走到本函数轮末）：步 11 编码按**内容**
//! 去重（`RecallView::contains_content`）⇒ 同一轮不会编出第二条；恢复幕的召回视图
//! 非空（首幕刚编的那条）⇒ 会落一条 `memory.recalled`。两条的溯源锚都指原轮用户格
//! （I2：恢复不新开用户格）。判据 = e2e `t27`（`turnFacts` 数轮次事实、派生事实另数）。
//!
//! 中止轮 / 等待轮的形态：用户格**先落**——prompt 从网格出，用户发言不入格就会从
//! 下一轮的 prompt 里消失；中止落收束格（`status: "aborted"` + `terminated`）、
//! 等待落进展格（`status: "pending"`），两条都不另立事件 kind。影响面：
//! 中止轮计入兜底率的**分母**（用户格在网格里），**分子不计**（中止不是失败）。

use std::sync::{Arc, Mutex};

use symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use symbio_core::{
    is_stop_fact, llm_short_id, run_ops, ActorSpec, AppendError, CapabilityMeta,
    ConsolidateRequest, DeltaSink, Entity, Event, ExecAbortSignal, ExecEventSink, FullModel,
    LatencyTier, LlmAdapter, ModelProvider, Plugin, PluginError, PluginInvokeRequest,
    ProviderLlmAdapter, Seq, Store, TokenIssuer, TurnOutput, Verb,
};
use symbio_plugin_api::events::{
    EVENT_AUTHZ_DENIED, EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL,
    EVENT_CHAT_ASSISTANT_PROGRESS, EVENT_USER_MESSAGE,
};

// 压缩入格与寄存补落都是**会话动作**（事实源归档不在 core `Store` 面内，写方必须
// 留在持令牌的 session）；本文件只剩轮内调用点，经 [`LoopSession`] 的端口动作发起。
use super::turn_runner::{TurnInput, TurnRunner, TurnSession, TurnSessionBegin};
// 运行器一族**已下沉到本插件**（2026-10-09，见 `turn_runner.rs` 的头注）：它们不是 core 的
// 契约面，生产消费方只有本插件，按 README §4 四问第 1 问该住这里。
use super::turn_session::LoopSession;

// —— 阶段拆分（· B2，DE-021 §5 动作 B2）——
// `execute_turn` 的四个处理阶段各自成文件；再导出保持 `turn_exec::*` 的既有路径不变
// （消费方与本文件测试都不必改引用）。阶段体是逐字搬移，本文件只留调度。
mod assemble;
mod call;
mod drive;
mod outcome;
mod persist;

pub(crate) use self::assemble::{assemble_turn, Assembled};
// 续写幕的开幕方（`chat_loop::drive`）要用同一个「已开未收束」判据：
// 这里的收束口径（只认 final / fallback）与 C4（兼认产物）刻意不同，见函数文档。
pub(crate) use self::assemble::last_open_turn;
pub(crate) use self::call::{open_turn, OpenRun, Running};
pub(crate) use self::drive::{drive, Control, DriveEnd, DriveStart};
// 夹具入口（幕已开好，只驱动）：生产不用它，只有测试的两个夹具从 crate 外借道。
// `stage_runtime` 是幕首之后那一半的同形入口——夹具自己备幕首，不该另抄一份
// 「分发方与汇报落地面怎么从装配束里造」。
#[cfg(test)]
pub(crate) use self::drive::{stage_runtime, ReadyStage, StageRuntime};
pub(crate) use self::outcome::{classify_turn, Classified, ExecTurnResult, Settled, TurnExecExit};
pub(crate) use self::persist::{execute_final_reply_turn, persist_turn};

/// UI 桥承载的节点类型：正文与推理**分属两条流**，各自一个节点，不合并。
///
/// 只有建节点（`Snapshot`）与定格（`Finalize`）需要它——窄追加（`Delta`）只带
/// `id + text`，归属由节点 id 决定（与 model 插件的帧面同一形状）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum UiNodeKind {
    Text,
    Reasoning,
}

impl UiNodeKind {
    /// 给这一帧的节点定形状：正文是 `Body`，推理是 `Notice`（折叠叶，[D-070 §决策 3]
    /// []——reasoning 与 compression 同格，前端渲染二者一样，业务区分只在 `style_tag`）。
    fn shape(self, node: ChatNode) -> ChatNode {
        match self {
            UiNodeKind::Text => node,
            UiNodeKind::Reasoning => {
                let mut n = node;
                n.msg_type = Some(NodeKind::Notice);
                n.with_style_tag(CHAT_STYLE_TAG_REASONING)
            }
        }
    }
}

/// UI 桥的帧：首片建节点（快照），后续窄追加，收束定格。语义与 model 插件的流式帧同构。
enum UiFrame {
    Snapshot {
        id: String,
        parent: String,
        text: String,
        kind: UiNodeKind,
    },
    Delta {
        id: String,
        text: String,
    },
    /// 收束本轮的节点（工具轮的中途正文 / 中途推理）：工具调用节点紧接着就来，
    /// 正文必须先在**同一个**节点上定格，否则它会一直转「流式中」。
    Finalize {
        id: String,
        parent: String,
        text: String,
        kind: UiNodeKind,
    },
    /// 屏障：收到即回执。给「通道外的发射方」一个**同步点**——
    /// 分发方（[`super::tool_dispatch::SessionDispatchPort`]）既走通道（定格正文）
    /// 又直接写出口（工具节点，`process_tool_calls_async` 只认 `ExecEventSink`），
    /// 两条路不排序就会让工具卡片跑到正文之前。
    Barrier(tokio::sync::oneshot::Sender<()>),
    /// 工具调用快照（身份帧）：**原样透传**的 `ChatNode`。
    ///
    /// 载体不止工具节点：对话面的**根级**节点（开场白）产生即上线时也借这一格——
    /// [`UiFrame::Snapshot`] 要求 `parent`，而开场白与用户发言互为兄弟，没有父。
    /// `DeltaSink::on_tool_frame` 的契约本就是「消费端不做类型推断、不改写身份」，
    /// 所以这里原样透传就是它的义务边界，不新增形状。
    ///
    /// 为什么不拆进 [`UiFrame::Snapshot`]：工具节点的 `name` / `tool_call_id` /
    /// 参数至今都是 model 插件转写面给出的事实，拆开再拼只会引入第二份形状；
    /// 分发方稍后以**同一 id** 定格与落库（`llm_build_tool_call_nodes` 的节点 id
    /// 就是转写面的节点 id），节点永远只有一张卡。参数窄增量**不走**本变体——
    /// 它走 [`UiFrame::Delta`]（按 id 合并的语义与正文增量共用一条）。
    Tool(Box<ChatNode>),
}

/// UI 帧队列的**容量上限**。
///
/// 取值不是性能参数而是**内存上界**：`on_delta` 是同步回调（不能 await），所以生产者
/// 不能被背压——慢消费者下唯一能让内存有界的办法是**给队列设界**，并在满了的时候
/// **合并增量**而不是丢帧（丢增量 = 丢正文，界面会少一截）。
const UI_FRAME_CAPACITY: usize = 64;

/// 有界 UI 帧队列：容量上限 + 增量合并。
///
/// ## 为什么不是 `mpsc` 加界就完事
///
/// 加了界必然要处理「满了怎么办」，而这里的答案是**不能丢**：
/// `Finalize` 丢一次，正文节点永远转在「流式中」；`Snapshot` 丢一次，那轮正文
/// 整段消失。所以本队列的策略是
///
/// - **增量（`Delta`）满了就合并**：并进队列里**同节点的最后一条增量**的正文。
///   这是**无损**的——增量本来就是追加，合并等价于「一批到达」；
/// - **结构帧（`Snapshot` / `Finalize` / `Barrier`）永不被丢**：满时先腾掉一条
///   增量；一条增量都腾不掉（队列里全是结构帧）才允许软超容，而结构帧每轮 O(1) 条，
///   在 64 的容量下不可能发生。
///
/// ## 为什么合并要「并进同节点的那一条」而不是「并进队尾」
///
/// 一轮的帧序是 `Snapshot → Delta* → Finalize`，而 `Finalize` 之后不会再有同节点的
/// 增量（节点已定格）。所以队列里同节点的增量必然**位于 Snapshot 与 Finalize 之间**，
/// 把新增量并进它既不丢文本也不打乱它与 Finalize 的相对次序。跨节点合并则会——
/// 那正是本实现要避免的。
///
/// ## 关闭语义
///
/// `UiBridge` drop 时关闭队列，接收端 `pop` 返回 `None` 自然退出——与原先
/// `UnboundedSender` drop 后 `recv` 返回 `None` 同形。
struct FrameQueue {
    inner: std::sync::Mutex<FrameQueueInner>,
    cap: usize,
    notify: tokio::sync::Notify,
}

#[derive(Default)]
struct FrameQueueInner {
    q: std::collections::VecDeque<UiFrame>,
    closed: bool,
    /// 因「队列里没有可腾的增量」而软超容的次数——恒应为 0，非零即说明容量取值有问题。
    soft_overflow: usize,
}

impl FrameQueue {
    fn new(cap: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(FrameQueueInner::default()),
            cap,
            notify: tokio::sync::Notify::new(),
        }
    }

    /// 入队一帧。返回 `false` 表示队列已关闭（接收端退出）——与 `UnboundedSender::send`
    /// 的 `SendError` 同形，调用方原有那几处 `let _ =` / `is_err()` 语义不变。
    fn send(&self, frame: UiFrame) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return false;
        }
        if g.q.len() < self.cap {
            g.q.push_back(frame);
        } else {
            match frame {
                UiFrame::Delta { id, text } => {
                    // 先找同节点的最后一条增量：找到就并进去（无损、保序）。
                    let merged =
                        g.q.iter_mut()
                            .rev()
                            .find_map(|f| match f {
                                UiFrame::Delta {
                                    id: fid,
                                    text: ftext,
                                } if *fid == id => {
                                    ftext.push_str(&text);
                                    Some(())
                                }
                                _ => None,
                            })
                            .is_some();
                    if !merged {
                        // 同节点一条都没有 ⇒ 这是该节点的首帧（队列里连它的
                        // Snapshot 都还没进），腾一条增量给它让位。
                        if let Some(pos) =
                            g.q.iter().position(|f| matches!(f, UiFrame::Delta { .. }))
                        {
                            g.q.remove(pos);
                        } else {
                            g.soft_overflow += 1;
                        }
                        g.q.push_back(UiFrame::Delta { id, text });
                    }
                }
                structural => {
                    // 结构帧不可丢：腾掉**最旧的一条增量**（不是队尾——队尾那条
                    // 可能正是定格前最后一段正文）。
                    if let Some(pos) = g.q.iter().position(|f| matches!(f, UiFrame::Delta { .. })) {
                        g.q.remove(pos);
                    } else {
                        g.soft_overflow += 1;
                    }
                    g.q.push_back(structural);
                }
            }
        }
        drop(g);
        self.notify.notify_one();
        true
    }

    /// 关闭队列并唤醒等待者（`UiBridge` drop 时调用）。
    fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.closed = true;
        drop(g);
        self.notify.notify_waiters();
    }

    async fn pop(&self) -> Option<UiFrame> {
        loop {
            // ⚠️ **必须先注册兴趣，再检查条件**——否则丢唤醒。
            //
            // 朴素写法（先查队列/closed、解锁、再 `notified().await`）在 `close()`
            // 恰好落在「解锁」与「await」之间时会**永远等下去**：那一刻还没有人在
            // 等待，`notify_waiters()` 唤不醒任何东西，而 `pop` 随后就注册上并
            // 睡下去，再没有第二次通知。
            //
            // 这个失效形态是**静默**的：不 panic、不超时、不占 CPU，只是永远不返回。
            // 它在本函数上真实发生过一次（见 `turn_exec.test.rs` 里 `drained` 的注释）。
            // `tokio::sync::Notify` 的规范解法就是这个顺序：先拿到 future、`enable()`
            // 登记兴趣，然后才去检查条件。
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut g = self.inner.lock().unwrap();
                if let Some(f) = g.q.pop_front() {
                    return Some(f);
                }
                if g.closed {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// 当前排队帧数（判据与测试用）。
    /// ⚠️ `#[cfg(test)]`：这两个是**判据用的观察口**，只有
    /// `turn_exec.test.rs` 调它们。生产构建里它们无人调用 ⇒ `clippy -D warnings`
    /// 判死代码。标 cfg(test) 而不是加 `dead-code-allow` 豁免——豁免是给「生产里
    /// 确实需要、只是暂时没人调」的东西用的，而这里的需求本来就不在生产侧。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.lock().unwrap().q.len()
    }

    /// 软超容累计（恒应为 0）。
    /// ⚠️ `#[cfg(test)]`：这两个是**判据用的观察口**，只有
    /// `turn_exec.test.rs` 调它们。生产构建里它们无人调用 ⇒ `clippy -D warnings`
    /// 判死代码。标 cfg(test) 而不是加 `dead-code-allow` 豁免——豁免是给「生产里
    /// 确实需要、只是暂时没人调」的东西用的，而这里的需求本来就不在生产侧。
    #[cfg(test)]
    pub(crate) fn soft_overflow(&self) -> usize {
        self.inner.lock().unwrap().soft_overflow
    }
}

/// 流式 UI 桥（DeltaSink 侧）：同步回调里只做「分帧 + 投递」——真正的
/// `emit`（异步）由执行任务里的接收端驱动（见模块文档）。
///
/// `pub(crate)`：工具分发方（[`super::tool_dispatch::SessionDispatchPort`]）要经
/// [`UiBridge::finalize_node`] 定格本轮正文——它必须与增量帧走**同一条通道**，
/// 否则定格帧会越过队列里未消化的增量先到，正文被追加到已定格的副本之后。
pub(crate) struct UiBridge {
    tx: Arc<FrameQueue>,
    root_id: String,
    /// 正文节点 `(id, 累积正文)`：首片分配 id、此后稳定；`finalize_node` 交还并置空
    /// （下一轮另开）。
    ///
    /// 正文也要**累积**（与推理同形）：`LlmTurn::text` 只在生成**完整结束**时才有，
    /// 而中止时那半截正文只有桥攒着——**半截正文是事实**，不入格/不落库就等于
    /// 用户中断后那半句话消失（追问名词就没了前置上下文）。累积是无损的：
    /// 每一片都先经这里再进队列，队列满了是**合并**同节点增量（见 `FrameQueue`），
    /// 不丢。
    node: Arc<Mutex<Option<(String, String)>>>,
    /// 推理节点 `(id, 累积正文)`：与正文同形。推理正文**没有别的持有者**——
    /// `LlmTurn` 只装 `text` 与 `tool_calls`，不在这里攒，收束帧就只能写
    /// 「推理到此为止」而不带内容，网格里也查不到 reasoning。
    reasoning: Arc<Mutex<Option<(String, String)>>>,
}

/// `UiBridge` drop 时**关闭队列**：接收端的 `pop` 因此返回 `None` 而退出。
///
/// 这条语义原先由 `UnboundedSender` 的 drop 免费提供——改成 `Arc<FrameQueue>` 后
/// 队列与桥同生共死，而**发射任务自己也持有一份 Arc**（`execute_turn` 里
/// `tx.pop()`），所以「桥没了通道就关了」不再自动成立。不显式关的话，发射任务
/// 会在每一轮结束后**永久挂起**：不 panic、不超时、不占 CPU，只是永不退出，
/// 而轮次照常增长——那种泄漏在日志里一个字都看不见。
impl Drop for UiBridge {
    fn drop(&mut self) {
        self.close();
    }
}

impl UiBridge {
    /// 关闭队列（幂等）：接收端的 `pop` 因此返回 `None` 而退出。
    ///
    /// 显式给出而不只靠 `Drop` 的原因：**桥可能比幕活得久**。幕的草稿里挂着
    /// `nodes` 闭包，它捕获着 `Arc<UiBridge>`，于是「这一幕收了」不等于「桥没了」
    /// ——收幕时要等发射任务消化完队列里剩下的帧（`emitter.await`），只靠 Drop
    /// 就会等到一个永不到来的关闭。
    pub(crate) fn close(&self) {
        self.tx.close();
    }

    /// 当前正文节点 `(id, 累积正文)`（未定格的最后一轮正文；本轮无正文 ⇒ `None`）。
    ///
    /// 累积正文的消费点有两个：`chat_loop` 的 `finalize_assistant_turn`（收口末轮）
    /// 与中止出口（半截正文入格，见 [`drive`]）。
    pub(crate) fn current_node(&self) -> Option<(String, String)> {
        self.node.lock().unwrap().clone()
    }

    /// 当前推理节点 `(id, 累积正文)`（未定格的最后一轮推理；本轮无推理 ⇒ `None`）。
    ///
    /// 与 [`Self::current_node`] 的分工：正文的末轮由 `chat_loop` 的
    /// `finalize_assistant_turn` 收口（那里拿着 `TurnOutput.text`），推理那侧同理
    /// 但正文得**从这里取**——`TurnOutput` 的 `reasoning` 字段正是由本函数回填的
    /// （见 [`drive`] 的收幕段）。
    pub(crate) fn current_reasoning(&self) -> Option<(String, String)> {
        self.reasoning.lock().unwrap().clone()
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
        let (id, _) = self.node.lock().unwrap().take()?;
        let _ = self.tx.send(UiFrame::Finalize {
            id: id.clone(),
            parent: self.root_id.clone(),
            text: text.to_string(),
            kind: UiNodeKind::Text,
        });
        Some(id)
    }

    /// 定格本轮推理节点并**重置**（返回 `(id, 累积正文)`；本轮无推理 ⇒ `None`）。
    ///
    /// 与 [`Self::finalize_node`] 同形、同理由（切节点 + 经通道），两处差别只有
    /// 全文的来源：正文由调用方给（`LlmTurn::text`），推理由桥自己攒（见 `reasoning` 字段）。
    pub(crate) fn finalize_reasoning_node(&self) -> Option<(String, String)> {
        let (id, text) = self.reasoning.lock().unwrap().take()?;
        let _ = self.tx.send(UiFrame::Finalize {
            id: id.clone(),
            parent: self.root_id.clone(),
            text: text.clone(),
            kind: UiNodeKind::Reasoning,
        });
        Some((id, text))
    }

    /// 等通道里**已排队**的帧全部落到出口（屏障）。
    ///
    /// 分发方在定格正文之后要直接写出口（工具调用节点、工具结果节点——它们由
    /// `process_tool_calls_async` 发出，那个函数只认 `ExecEventSink`）。通道里的
    /// 定格帧此刻可能还没被发射任务消化，不设同步点就会**倒序**：工具卡片先到，
    /// 正文后到，前端按到达顺序排出来的时间线是错的。
    pub(crate) async fn flush(&self) {
        let (done, wait) = tokio::sync::oneshot::channel();
        if !self.tx.send(UiFrame::Barrier(done)) {
            return; // 接收端已退出（生成结束）：没有待落帧可言。
        }
        // 屏障没走完 = **同步点失效**，不是「无所谓的失败」：定格帧还压在队列里，
        // 而调用方（`tool_dispatch::dispatch` 的 ①）紧接着就把工具节点**直接**写出口——
        // 那正是本函数存在的理由（见那里的注释：工具卡片会跑到正文之前）。
        //
        // 接收端只在两种情况下丢掉 `done`：emitter 任务被 abort，或它 panic；两者都
        // 意味着本轮 UI 已经没了，没有可重试的对象。所以只记一笔、不冒泡——但**不许
        // 静默**：静默的失效与「没有这个同步点」在日志里长得一模一样，下次有人把它
        // 一起删掉也不会有人察觉。`grep-audit` 的 S-002-bonus 因此判它是业务路径吞错。
        if let Err(e) = wait.await {
            symbio_core::plugin_warn!(
                "session",
                "[v2] UI 桥屏障未完成（emitter 已退出：{}）⇒ 定格帧可能未落出口，时序无法保证",
                e
            );
        }
    }
}

impl DeltaSink for UiBridge {
    fn on_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        // 取 id、累积与建帧**一次锁内做完**（与 [`Self::on_reasoning`] 同形、同理由：
        // 三个动作必须原子，分两次锁会让「首片」的判定与累积不同步）。帧在锁内
        // 建好、`send` 在锁外——`send` 会去开队列自己的锁，没必要叠着。
        let frame = {
            let mut guard = self.node.lock().unwrap();
            match guard.as_mut() {
                Some((id, acc)) => {
                    acc.push_str(text);
                    UiFrame::Delta {
                        id: id.clone(),
                        text: text.to_string(),
                    }
                }
                None => {
                    let id = llm_short_id();
                    *guard = Some((id.clone(), text.to_string()));
                    UiFrame::Snapshot {
                        id,
                        parent: self.root_id.clone(),
                        text: text.to_string(),
                        kind: UiNodeKind::Text,
                    }
                }
            }
        };
        // 有界队列：入队不阻塞（`on_delta` 是同步回调，不能 await），满了合并
        // 增量而不是丢——丢增量等于丢正文。见 `FrameQueue`。
        let _ = self.tx.send(frame);
    }

    /// 推理增量：与 [`Self::on_delta`] **同形**（首片建节点、此后窄追加 + 累积正文），
    /// 只差节点类型是 `Reasoning`。
    fn on_reasoning(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        // 取 id、累积与建帧**一次锁内做完**：三个动作必须原子（同一片既进累积
        // 又定帧序）。分两次锁会让「首片」的判定与累积不同步，且首片那侧还得
        // 再锁一次把刚写进去的 id 取回来（`as_ref().unwrap()` 那种写法）。
        // 帧在锁内建好、`send` 在锁外——`send` 会去开队列自己的锁，没必要叠着。
        let frame = {
            let mut guard = self.reasoning.lock().unwrap();
            match guard.as_mut() {
                Some((id, acc)) => {
                    acc.push_str(text);
                    UiFrame::Delta {
                        id: id.clone(),
                        text: text.to_string(),
                    }
                }
                None => {
                    let id = llm_short_id();
                    *guard = Some((id.clone(), text.to_string()));
                    UiFrame::Snapshot {
                        id,
                        parent: self.root_id.clone(),
                        text: text.to_string(),
                        kind: UiNodeKind::Reasoning,
                    }
                }
            }
        };
        let _ = self.tx.send(frame);
    }

    /// 工具调用帧（快照 / 参数窄增量，见 [`DeltaSink::on_tool_frame`]）。
    ///
    /// 与 [`Self::on_delta`] 的差别：节点 id **不由本桥铸造**——快照帧的 id /
    /// name / tool_call_id / 参数是 model 插件转写面的事实，分发方稍后以同一 id
    /// 定格与落库；本桥只负责把帧送进**同一条**队列（与正文共用容量与屏障语义）。
    fn on_tool_frame(&self, m: &ChatNode) {
        match &m.delta {
            // 参数窄增量：并进与正文同款的 Delta 帧——队列满时按 id 合并的
            // 无损语义因此对工具参数同样成立（不会因换了通道就退化成丢帧）。
            Some(d) => {
                let _ = self.tx.send(UiFrame::Delta {
                    id: m.id.clone(),
                    text: d.clone(),
                });
            }
            // 快照（身份帧）：原样透传。
            None => {
                let _ = self.tx.send(UiFrame::Tool(Box::new(m.clone())));
            }
        }
    }
}

/// 一轮 v2 执行的入参包（与 [`TurnInput`] 同族：职责不同的多项不摊成
/// 长参数表——`too_many_arguments` 的根因是「一个函数管太多件事」，
/// 捆包让每项在调用点自带名字）。
///
/// 刻意**不持有 `ChatOrchestrator`**：那会把「执行一轮」与「编排器」绑死，
/// 而这里需要的只是它的三项（模型服务 / 插件宿主 / 本插件目录）——捆成窄接口
/// 之后，单测不必造一个完整编排器。
/// 抽干口：返回一批补充（正文 + 原条数），或 `None` = 没有。
///
/// 由 `chat_loop` 从会话端口的 `take-supplements` 动作适配而来（队列与配置都在
/// session，那个类型不进 `ExecTurn`）。**异步**是因为动作面本身是异步的；闭包返回
/// boxed future 而不是让调用点各自 spawn，保持「一次抽一批」的语义在同一个调用栈里。
/// 测试直接给闭包，于是能造「只返回一次」这种形状。
pub(crate) type SupplementFn = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<symbio_core::DrainedSupplement>> + Send>,
        > + Send
        + Sync,
>;

pub(crate) struct ExecTurn<'a> {
    /// 会话窄句柄（交付包 + 动作面端口；不持有会话存储实现）。
    ///
    /// 借的是 `Arc` 而不是 `&LoopSession`：幕首之后装配结果要**按值**活过整幕
    /// （控制面只有一个环，幕与幕在同一次调用里接续），借来的引用活不到那一刻；
    /// 而 `Arc` 只是句柄的克隆，仍然是同一个实例。
    pub session: &'a Arc<LoopSession>,
    /// 本轮生效的模型服务（core trait 的 trait object）。
    pub provider: Arc<dyn ModelProvider>,
    /// 插件宿主（工具能力经它路由；无工具轮为 `None`）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本轮请求上下文（工具路由与能力注册表都在它上面）。
    pub ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    /// 会话级系统提示词（取值点 = `chat_loop::inputs::collect_reads`）。
    pub system_prompt: &'a str,
    /// 本轮中止信号（同样由 chat_loop 传入）。
    pub abort: ExecAbortSignal,
    /// 本轮 Turn 根节点 id（UI 帧挂载的父节点）。
    pub root_id: &'a str,
    /// 一幕一个根：`true` = 本幕现分配的根（发骨架），`false` = 复用被续写轮的根。
    pub fresh_root: bool,
    /// 事件出口（UI 帧经此回到对话面）。
    pub sink: &'a ExecEventSink,
    /// 本轮用户发言（反射档的技能命中判定与轮末学习用它；事实与 prompt 都从
    /// 用户格出，不再经过这里）。
    pub user_text: &'a str,
    /// 本轮 turn 号与溯源锚：由 `chat_loop::drive` 在判决之前定下（用户格先落）。
    pub turn_no: u64,
    pub user_seq: u64,
    /// 本幕的锚（新轮 = 用户格 seq；续写幕 = 恢复阶段事实 seq）。
    pub drive_seq: u64,
    /// 对话窗口（最近轮数，含当前轮）。
    pub window_turns: u64,
    /// 本轮装配进模型的工具清单（空 = 无工具轮）。
    pub tools: &'a [CapabilityMeta],
    /// 补充抽干口（缺口 3）：工具循环每跑完一轮问一次「用户有没有中途插话」。
    /// 有就折进**本轮**——落一格 `turn.supplemented` 事实 + 进下一次请求。
    ///
    /// **闭包而不是具体类型**：抽干的实现是 `SupplementDrain`（要 config / change_subs / 会话
    /// 状态），而这里只需要「取一批」这一个动作。绑具体类型的话，**测试造不出假件**
    /// ——那等于这条链路没法在单测里验，只能靠 e2e，而 e2e 照不出「落格带没带溯源」
    /// 「`count` 对不对」这种细节。
    ///
    /// 类型定义在插件侧（core 不参与这套，见 `TurnInput::inject`）。
    pub supplements: Option<SupplementFn>,
    /// 轮中汇报（缺口 4）的**能力口**：`None` = 这一幕的 compose 产线关着
    /// （`compose_enabled = false`）——那一行到期时执行体交不回话，只落一格
    /// `result:"silent"` 的尝试，这一段静默因此不再重复到期。
    ///
    /// 与 [`Self::supplements`] **共用同一个挂点**（`TurnInput::inject`）：core 只知道
    /// 「一轮跑完了，问一次」——它不认得「汇报」这个业务词。
    ///
    /// 交下来的是**能力**（产一句什么话）而不是决定（要不要说）：后者已在行表里
    /// （`policy.report` + `report_due`）。落格与上线在本侧，编排器与 compose
    /// 在那一侧——两边各管一半（D-163）。
    pub report: Option<super::report_exec::ReportPort>,
    /// 本轮**轮内注入事件**的序号游标（事件 id `s-{turn}-{no}` / `rp-{turn}-{no}`
    /// 的那一段）。
    ///
    /// **必须是共享可变的**（`Arc<AtomicU64>`）：注入闭包是 core 在循环里调的，
    /// 而闭包只捕获它自己那份——不共享的话第二次注入会用同一个 id，撞幂等键。
    ///
    /// 补充与汇报**共用**这一支游标：两者都在同一个轮边界闭包里、都是
    /// `turn × asserted`，事件 id 前缀已把它们分开；分开两支游标只会多一份要
    /// 同步的状态（那才是真源变两处的形状）。
    pub supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    /// **本轮建树策略**（`SessionConfig::presentation_tree` 的请求级快照）。
    ///
    /// 与 v1 流式（`ChatOrchestrator::presentation_tree`）**同一个取值快照**——一次
    /// 请求里 v1、v2 必须按同一档建树，否则 live↔存储↔重建三格错位（树形状锁步）。
    /// 由 `chat_loop` 从编排器带入，本函数不读配置（配置在请求生命周期内不变，
    /// 取一次即够）。
    pub presentation_tree: symbio_core::node_build::PresentationTree,
}

/// 一次执行体动作需要的**全部**环境（D-222 裁决 5 的 `Ctx`）：轮内借面 + 轮首借面。
///
/// 它是派发器与执行体之间唯一的界面：执行体拿得到草稿与端口，拿不到第二份草稿，
/// 也拿不到「下一个动作是什么」的选择权（D-118 决策 4）。
///
/// ## 为什么轮首与轮内合成一个 `Ctx`
///
/// 判债只有一个循环（[`drive`]），而它选中的行可能来自轮首、接缝或内环——绑定表合表
/// 之后，「这一行借什么」不再是派发点的形状差别，只是 `Ctx` 里**哪一片**有值。轮首
/// 那几行与接缝读 [`HeadCtx`]；轮内那几行只读上半片。`head = None` 的调用点（定稿
/// 答话轮在 `prepare` 就收束，活不到这里）因此仍然可以驱动，只是轮首行报读失败。
///
/// ## 端口为什么是 `Arc` 而不是 `&`
///
/// 一次模型往返的 future 要能**离开它诞生的那一层栈帧**——派出与收应答分处两个环步
/// （`DE-015` 的 `D-222` 订正 2），future 得寄存进本轮草稿。future 捕获的每一样都必须是
/// owned，否则它的类型里会带一个只在那层栈帧里成立的生命周期，寄存进草稿就成了自引用
/// （而带生命周期的草稿会撞上 `Action` 的 `for<'a>`——绑定表要求动作是 `'static`）。
/// 于是这一层把「一次派发要用的端口」全部变成可克隆的句柄：`Arc<dyn Store>` /
/// `Arc<dyn LlmAdapter>` / `Arc<[CapabilityMeta]>` / `Arc<dyn DeltaSink>`，以及零大小的
/// [`FullModel`]（`Copy`）。
///
/// 仍借的只有三样：`session`（本轮草稿，派发器持有）、`dispatch`（工具分发方，它自己
/// 就在派发动作的栈上——工具批的等与收本来就在同一个动作里，不进 future），以及
/// `head`（轮首那一片可控状态，按步借、按步还）。
pub(crate) struct Ctx<'a> {
    pub(crate) session: &'a mut TurnSession,
    pub(crate) store: &'a Arc<dyn Store<Event = Event>>,
    pub(crate) llm: Arc<dyn LlmAdapter>,
    pub(crate) tok: FullModel,
    pub(crate) sink: Arc<dyn DeltaSink>,
    pub(crate) tools: Arc<[CapabilityMeta]>,
    pub(crate) dispatch: Option<Arc<dyn super::tool_dispatch::ToolDispatch>>,
    /// 轮首借面：`Some` = 这一幕有编排层那一片状态（判决 / 压缩 / 召回 / 接缝 / 汇报
    /// 可用）。轮内那几行不读它，`None` 对它们没有影响。
    pub(crate) head: Option<HeadCtx<'a>>,
}

/// 轮首那一片可控状态（一个动作之内借出、还回）。
///
/// 与 [`Ctx`] 的上半片分开成形，因为派发器要在每一圈**重新借**：`context` / `turn`
/// 是 `&mut`，出借方（[`HeadParts`]）持有整幕的借用权，逐圈把可变引用再借一次给
/// 一个动作。两片合在 `Ctx` 里只是为了让「这一行借什么」由数据形状说，不由派发点说。
pub(crate) struct HeadCtx<'a> {
    pub(crate) orchestrator: &'a super::chat_loop::state::ChatOrchestrator,
    pub(crate) request_ctx: &'a Arc<dyn symbio_core::PluginInvokeRequest>,
    /// 会话窄句柄：压缩 / 补落寄存 / 工具存档目录这些**会话动作**的入口。
    pub(crate) loop_session: &'a LoopSession,
    pub(crate) context: &'a mut super::chat_loop::state::SessionContext,
    pub(crate) turn: &'a mut super::chat_loop::state::TurnState,
    /// 请求级配置快照：续幕要按同一份配置重落 `session.config`。
    pub(crate) req: &'a super::chat_loop::state::TurnRequest,
    pub(crate) bridge: &'a Arc<UiBridge>,
    pub(crate) event_sink: &'a ExecEventSink,
    pub(crate) root_id: &'a str,
    pub(crate) fresh_root: bool,
    /// 本幕主体（幕首已定）：接缝的派发格与首响格都要它，重算就是第二个取值点。
    pub(crate) actor: &'a ActorSpec,
    pub(crate) report_port: Option<&'a super::report_exec::ReportPort>,
    pub(crate) report_landing: &'a super::report_exec::ReportLanding,
}

/// [`drive`] 手里的轮首出借方：整幕持有，逐圈把可变引用再借给 [`HeadCtx`]。
///
/// 两半的寿命不同：`context` / `turn` / `req` 是**借的**（出借方是调用方，且每圈
/// 要重新借一次），其余全是**owned 的句柄**（桥 / 发射口 / 根 id / 主体 / 汇报面）。
/// 后半片之所以按值带着，是因为它的来源——装配束——随幕的收束被拆开消费：借它的话
/// 归还之后就没有出借方了。这与 [`Ctx`] 把端口做成 `Arc` 是同一条理由。
pub(crate) struct HeadParts<'h> {
    pub(crate) orchestrator: Arc<super::chat_loop::state::ChatOrchestrator>,
    pub(crate) request_ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    pub(crate) loop_session: Arc<LoopSession>,
    pub(crate) context: &'h mut super::chat_loop::state::SessionContext,
    pub(crate) turn: &'h mut super::chat_loop::state::TurnState,
    pub(crate) req: &'h super::chat_loop::state::TurnRequest,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) event_sink: ExecEventSink,
    pub(crate) root_id: String,
    pub(crate) fresh_root: bool,
    pub(crate) actor: ActorSpec,
    pub(crate) report_port: Option<super::report_exec::ReportPort>,
    pub(crate) report_landing: super::report_exec::ReportLanding,
}

impl<'h> HeadParts<'h> {
    /// 逐圈把出借方手里的 `&mut` 再借给一个动作（`D-222` 裁决 5 的「按步获取、
    /// 按步释放」）：动作跑完，这些借用随 [`HeadCtx`] 一起还回来。
    pub(crate) fn as_ctx(&mut self) -> HeadCtx<'_> {
        HeadCtx {
            orchestrator: self.orchestrator.as_ref(),
            request_ctx: &self.request_ctx,
            loop_session: self.loop_session.as_ref(),
            context: &mut *self.context,
            turn: &mut *self.turn,
            req: self.req,
            bridge: &self.bridge,
            event_sink: &self.event_sink,
            root_id: &self.root_id,
            fresh_root: self.fresh_root,
            actor: &self.actor,
            report_port: self.report_port.as_ref(),
            report_landing: &self.report_landing,
        }
    }
}

/// 动作面的失败种类（D-123 决策 3，按 D-119 决策 1 落在**部署层**而不是机制层）。
///
/// 它是执行体与派发器之间唯一的失败通道：从此扩展失败种类**只能靠加变体**，而每加
/// 一个变体都要回答「派发器为它落哪条事实」——`String` 版失败会把这件事藏进文本。
///
/// 答案写在**同一个派发器**里（[`drive`]）：合成一个循环之后，失败种类
/// 不再按行分叉——「派发器为它落哪条事实」由守卫与账本说。
///
/// 为什么不进 `symbio_core`：机制层既不产它也不读它（`ring::drive` 只裁决欠不欠，
/// `run` 的指针按 D-119 本来就留在部署层），放进 core 就是一个**单消费方导出**，
/// 正是 `core-export-audit` C-003 要往下沉的那种东西——判据的实质（不是 `String`、
/// 加变体才扩展）与类型住在哪一层无关。
#[derive(Debug)]
pub(crate) enum ActorError {
    /// 落格失败（幂等键撞了、写侧闸不过、存储出错）。
    Append(AppendError),
    /// 动作要读的那一侧没交出它要的东西（会话端口的一次读失败）。
    ///
    /// 「派发器为它落哪条事实」的答案按 host 分：**轮内今天产不出这一档**——模型与
    /// 工具往返的失败都经端口落成 `failed(port=…)`，由账本判，不经返回值；**轮首**的
    /// 读失败发生在幕锚之后、任何请求格之前，端口面上没有一笔账可关，因此交回编排层
    /// 走 `finish_turn(TurnExit::Failed)` 的兜底格既有路径（与换成动作通道之前逐字
    /// 同行为）。
    Read(symbio_core::PluginError),
}

/// 动作的返回：`Ok(())` = 这一步做完了（本轮是否收束由派发器读账本判），`Err` = 失败。
///
/// **动作不交回产出**（D-123 决策 5）：收束的判据在日志上（账本里出现停机格），
/// 产出由收束那一步寄存在本轮草稿（[`prepare::Prepared::settled`]），派发器收工时取走。
/// 让它交回 `Option<TurnOutcome>` 等于把「本轮结束了没有」的判断权还给执行体。
///
/// `+ Send` 不是装饰：插件的 `dispatch` 未来要交回线程池，派发器在它里面 await
/// 这个动作。
///
/// 这一条返回通道只有一种（`D-222` 裁决 5）：「这一步做完了 / 失败了」与「产出不由
/// 返回值交回」是同一句话，轮首行与轮内行共用它。
pub(crate) type StepFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ActorError>> + Send + 'a>>;

/// 绑定表里的动作类型。**一个 `by` 一个动作**：派发器只调它，不按行名 / 端口名 /
/// 业务语义分支（D-118 决策 1–2；`Executor::for_by` 的名字 match 已退役）。
///
/// 借用只有一条寿命：`Ctx` 的端口字段全是 `Arc`，借来的东西是草稿与轮首那一片
/// （派发器持有并逐圈再借）。这就是 §2.3 草图里 `'static` 换不来的那条——草稿必须
/// 被借走，动作就不能是 `'static`（D-119）。
///
/// 轮首与轮内**只有这一种签名**（`D-222` 裁决 5）：合成一个循环之后，
/// 「这一行借什么」由 [`Ctx::head`] 有没有值说，不由函数类型说。
pub(crate) type Action = for<'a> fn(&'a mut Ctx<'a>) -> StepFuture<'a>;

/// `execute` 执行体：**宣布**一次模型往返（`request(port=model)`）。
///
/// 派出归 [`run_model`]（`dispatch-model` 行的 `relay` 执行体，D-166）：
/// 「这一笔该谁跑」由行表选债决定，不由这段代码顺手调一次往返决定。两个动作之间
/// 唯一的交接件是草稿里的 `pending_round`。
///
/// 工具批的三笔债（创建 / 授权 / 派发）已各自成行（b），不再在这个
/// 执行体里按草稿分支：[`super::turn_exec::run_open`] / `run_authorize` /
/// `run_dispatch` 各由自己的行选中。
///
/// 收束**不归这里**：应答落地后 `close-turn`（55）比这一行（60）先被选中，入格
/// 归 `close` 执行体。审批 / 问答恢复也不再旁路收束（d）：唤醒请求进
/// 行表之后，恢复幕与普通幕走同一组行。
///
/// 步数在 [`TurnSession::announce_model`] 里**记**（口径 = 模型往返次数，
/// 与拆分前「一步 = 一轮往返」同），上界判断在派发器——停机判据不搬进执行体
/// （D-118 决策 7）。
pub(crate) fn run_execute<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .announce_model(ctx.store.as_ref())
            .await
            .map_err(ActorError::Append)
    })
}

/// `relay` 执行体（D-166）：`dispatch-model` 行选中的动作——把**已宣布**的那一次
/// 模型往返发出去，并把结局归位。
///
/// 它不判断「该不该发」：那一半由行表的债说（`request` 开着才轮得到它）；
/// 它也不判断「这一轮结束没有」：结局寄存进草稿，收束归 `close` 执行体
/// （D-123 决策 5）。
pub(crate) fn run_model<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .begin_model(
                ctx.store.clone(),
                ctx.llm.clone(),
                ctx.tok,
                ctx.sink.clone(),
                ctx.tools.clone(),
            )
            .await
            .map_err(ActorError::Append)
    })
}

/// `waiter` 执行体（`D-222` 订正 2）：`await-model` 行选中的动作——取走寄存的那一次
/// 往返，等它落地，落 `response` / `failed`。
///
/// 与 [`run_model`] 成对：派出（`dispatch-model`，出口是 `sent`）与收应答
/// （`await-model`，出口是 `response` / `failed`）分处两个环步。两次选债之间那一圈
/// 「已派出、未收清」因此在事实面上可见——`waiting` 视图与 `reap` 的 `stale_sent`
/// 读的都是它，而不是只在崩溃恢复的输入上才有值。
pub(crate) fn run_await<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .await_model(ctx.store.as_ref(), ctx.llm.as_ref(), ctx.dispatch.is_some())
            .await
            .map_err(ActorError::Append)
    })
}

/// `opener` 执行体（b）：`open-tools` 行选中的动作——逐条开账。
pub(crate) fn run_open<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .open(ctx.store.as_ref(), ctx.dispatch.as_deref())
            .await;
        Ok(())
    })
}

/// `authorizer` 执行体（b）：`authorize-tool` 行选中的动作——判放行 / 拒绝。
pub(crate) fn run_authorize<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .authorize(ctx.store.as_ref(), ctx.dispatch.as_deref())
            .await;
        Ok(())
    })
}

/// `courier` 执行体（b）：`dispatch-tools` 行选中的动作——把放行的每一笔**交出去**
/// （落 `sent`），批次的收尾归 [`run_finish_tools`]。
///
/// 与 [`run_finish_tools`] 分开是 `D-124` 订正 1 的那一刀：派出与收应答分处两个环步，
/// 两次选债之间那一圈「已派出、未收清」才是 `waiting` 视图在**正常跑**里读得到的形状。
pub(crate) fn run_begin_tools<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .begin_tools(ctx.store, ctx.dispatch.clone())
            .await;
        Ok(())
    })
}

/// `waiter-tools` 执行体（`D-124` 订正 1）：`await-tools` / `deny-tools` 两行选中的
/// 动作——收这一批的结果并关账。
pub(crate) fn run_finish_tools<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .finish_tools(ctx.store, ctx.llm.as_ref(), ctx.dispatch.clone())
            .await
            .map_err(ActorError::Append)
    })
}

/// `close` 执行体：把寄存的那份**结局**落成收束格（/ 步 5c）。
///
/// 它跑不跑由**日志**说（`close-turn` 行的前件 = `turn.closeable`：答完了 **或**
/// 模型没答成），不由草稿说：与 [`run_execute`] 的分工是「谁欠什么」在表里、
/// 「怎么还」在动作里。不记步数——收束不是一次模型往返，步数口径与拆分前一致
/// （D-115 决策 3）。
///
/// 两条收束格都在它名下（`chat.assistant.final` / `chat.assistant.fallback`）：
/// `execute` 那一步只跑模型往返，收束格一律由这里落（中止 / 适配器失败两条路径
/// 因此从执行体内部挪到了收束动作里）。
pub(crate) fn run_close<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        ctx.session
            .close(ctx.store.as_ref(), ctx.llm.as_ref(), ctx.sink.clone())
            .await
            .map_err(ActorError::Append)
    })
}

/// 轮首那一组的**读失败**：动作需要 [`HeadCtx`]，而这一台的调用点没给。
///
/// 今天只有定稿答话轮能到这一格，而它在 `prepare` 的定稿答话分支就收束了（驱动不被调用）。
/// 写成显式错误而不是 `unwrap`：真到了这里，事实面上没有可关的账，交回编排层比
/// 静默空转好（与 [`ActorError::Read`] 的语义一致）。
fn head_missing() -> ActorError {
    ActorError::Read(PluginError::InternalError(
        "这一行需要轮首环境（编排层状态），而本次驱动没有给出".into(),
    ))
}

/// 取轮首借面（一个动作之内独占）。
fn take_head<'a>(ctx: &mut Ctx<'a>) -> Result<HeadCtx<'a>, ActorError> {
    ctx.head.take().ok_or_else(head_missing)
}

/// `classify` 执行体：`policy.classify` 行选中的动作。
///
/// 本体留在它的领域层（[`super::chat_loop::run_classify`]），这里只把它指到名字上。
pub(crate) fn run_classify<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        super::chat_loop::run_classify(
            head.orchestrator,
            head.request_ctx,
            ctx.store,
            head.context,
            head.turn,
        )
        .await
    })
}

/// `recall` 执行体：`policy.recall` 行选中的动作。
///
/// 本体留在它的领域层（[`super::chat_loop::run_recall`]——记忆 / 技能 / 调度段
/// 三段读视图都住在那里），这里只把它指到名字上并交出出口那一格。
pub(crate) fn run_recall<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        super::chat_loop::run_recall(
            head.request_ctx,
            ctx.store.as_ref(),
            head.context,
            head.turn,
        )
        .await
    })
}

/// `compress` 执行体：`policy.compress` 行选中的动作。
///
/// 本体留在它的领域层（[`super::chat_loop::run_compress`]），水位要的那份请求级
/// 开销读本轮状态的 `request_overhead`。
pub(crate) fn run_compress<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        super::chat_loop::run_compress(
            head.orchestrator,
            head.request_ctx,
            ctx.store,
            head.context,
            head.turn,
        )
        .await
    })
}

/// `reflex` 执行体：`policy.reflex` 行选中的动作（D-163）。
///
/// 它只落「反射档已答」那一格，并把技能正文寄进本轮状态；收束格由 `close-turn`
/// 统一落——快路自身再写 `final` 会让依赖图成环。
pub(crate) fn run_reflex<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        super::chat_loop::run_reflex(
            head.request_ctx,
            ctx.store.as_ref(),
            head.context,
            head.turn,
        )
        .await
    })
}

/// 接缝执行体：`prepare-turn` 行选中的动作（`D-223` 订正 1 / 订正 2）。
///
/// 轮首那几行跑完之后、任何请求之前的一次性装配。次序是硬的：
///
/// 1. 压缩入格 + 归档——早于水位（量的是压过的那一体）、早于骨架（压不动时不该留
///    半截 Turn 根）、早于派发格（`consolidation_is_forwarded_before_the_turn_runs`）；
/// 2. 水位 + 根骨架 + 请求视图——三者都要轮首那几行寄存下来的值；
/// 3. `inputs` 格（`turn_head.prepared`）与重试分支（根 id 已定）；
/// 4. 定档 + 首响——两格都要等轮首那几行（档位折自 `policy.reflex` 的出口格）；
/// 5. 基线 prompt / 前缀 / 快路正文——基线必须在压缩之后渲染。
pub(crate) fn run_seam<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        let consolidation = head.context.consolidated.take();
        let opening_text = head.turn.opening_text.take();
        let opening_dialog = head.turn.opening_dialog.take();
        self::assemble::land_consolidation(head.loop_session, consolidation.as_deref()).await;
        let sections = super::chat_loop::inputs::prepare_request_sections(
            head.orchestrator,
            head.request_ctx,
            head.context,
            head.event_sink,
            head.turn,
            head.root_id,
            head.fresh_root,
            ctx.tools.as_ref(),
        )
        .await?;
        land_inputs(
            ctx.store.as_ref(),
            head.request_ctx.as_ref(),
            head.turn,
            "ready",
        )?;
        land_retry_branch(
            ctx.store.as_ref(),
            head.request_ctx.as_ref(),
            head.turn,
            head.root_id,
        )?;
        self::assemble::land_seam(
            ctx.store.as_ref(),
            head.bridge,
            head.turn,
            head.actor,
            opening_text.as_deref(),
            opening_dialog.as_ref(),
        )
        .await?;
        ctx.session.render_base(ctx.store.as_ref());
        ctx.session
            .set_prefix(super::chat_loop::request_view_prefix(&sections));
        ctx.session.set_reflex(head.turn.head_reflex.clone());
        Ok(())
    })
}

/// `report` 执行体：`policy.report` 行选中的动作——取运行现状 → 要一句措辞 →
/// 落尝试格（+ 汇报事实）→ 上线。
///
/// ## 读数从视图来
///
/// `quiet_ms` / `tool_rounds` / `anchor` 都是 `report_due` 的输出：同一份日志、同一个
/// `now` 在选债与执行两处折出同一份值。让派发点把值递进来会造出第二个读数点。
///
/// ## 尝试格先于汇报事实
///
/// 汇报事实自己是下一段静默的锚（`is_report_anchor`）。尝试格若排在它之后，
/// `report_due` 会把下一段静默也判成「试过了」——一段之后再也不报。所以先落尝试格，
/// 再落那句事实。
///
/// ## 拿不到措辞 ⇒ 只落尝试格
///
/// `compose` 未挂载 / 模板产线失败时交回 `None`：这一笔账照旧开着，但**这一段静默
/// 不再重试**（`silent` 就是「试过了、没话」）。下一个批次边界是新的静默段、新的一笔。
pub(crate) fn run_report<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        let turn_no = head.turn.turn_no;
        let principal = symbio_core::request_principal(head.request_ctx.as_ref());
        let facts = ctx.store.range(Seq::new(0));
        let Some(view) = super::loop_rows::view("report_due", &facts, turn_no, head.turn.now_ms)
        else {
            return Err(ActorError::Read(PluginError::InternalError(
                "轮中汇报要读 `report_due`，而声明层没交出这个视图".into(),
            )));
        };
        let anchor = view.get("anchor").and_then(|v| v.as_u64()).unwrap_or(0);
        let quiet_ms = view.get("quiet_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let tool_rounds = view
            .get("tool_rounds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let body = match head.report_port {
            Some(port) => {
                (port.text)(symbio_core::RunSnapshot {
                    tool_rounds,
                    quiet_ms,
                })
                .await
            }
            None => None,
        };
        let result = if body.is_some() { "sent" } else { "silent" };
        super::stage_events::land(
            ctx.store.as_ref(),
            turn_no,
            head.turn.user_seq,
            &principal,
            &[super::stage_events::StageFact::report(
                head.turn.request_round,
                anchor,
                result,
            )],
        )
        .map_err(ActorError::Append)?;
        let Some(body) = body else {
            return Ok(());
        };
        let draft = symbio_core::facts::ReportDraft {
            text: body,
            tool_rounds,
            quiet_ms,
        };
        let node = draft.to_message(&symbio_core::principal::Principal::from_legacy(&principal));
        ctx.store
            .append(
                symbio_core::Event::pending(
                    format!(
                        "rp-{turn_no}-{}",
                        head.report_landing
                            .no
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ),
                    symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED,
                    Entity::Turn,
                    Verb::Asserted,
                    turn_no,
                    &principal,
                )
                .with_produced_by(anchor)
                .with_payload(serde_json::json!({
                    "text": draft.text,
                    "tool_rounds": draft.tool_rounds,
                    "quiet_ms": draft.quiet_ms,
                    // 节点身份是事实的一部分：重建侧按它复刻同一张卡。
                    "id": node.id.clone(),
                })),
            )
            .map_err(ActorError::Append)?;
        if let Ok(mut sink) = head.report_landing.collected.lock() {
            sink.push(node.clone());
        }
        head.report_landing.live.on_tool_frame(&node);
        Ok(())
    })
}

/// `continue` 执行体（`D-222` 裁决 2）：`continue-turn` 行选中的动作——同一轮再接一幕。
///
/// 它只做开幕那一套里「不新开用户格」的那一半：落新幕的 `round` 格（那一格同时是
/// 新幕的锚）、收租、重落配置。新幕欠什么由新幕自己的事实说——本动作不判流向、
/// 也不再起一个环（裁决 5：控制面只有一个环）。
pub(crate) fn run_continue<'a>(ctx: &'a mut Ctx<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let head = take_head(ctx)?;
        super::chat_loop::continue_drive(head.context, head.turn, head.req)
            .map(|_| ())
            .map_err(|exit| match exit {
                super::chat_loop::state::TurnExit::Failed(e) => ActorError::Read(e),
                other => ActorError::Read(symbio_core::PluginError::InternalError(format!(
                    "续幕的开幕交出了非失败结局：{other:?}"
                ))),
            })
    })
}

/// 把「备输入」的出口落成阶段事实（`turn.stage{stage:"inputs"}`）。
///
/// `result` 二值：`ready` = 这一幕要跑（请求视图已备好）；`closed` = 定稿答话轮在
/// 轮首就收束了。它是 `turn_head.prepared` 的唯一来源。
pub(crate) fn land_inputs(
    store: &dyn Store<Event = Event>,
    ctx: &dyn PluginInvokeRequest,
    turn: &super::chat_loop::state::TurnState,
    result: &str,
) -> Result<(), ActorError> {
    super::stage_events::land(
        store,
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx),
        &[super::stage_events::StageFact::inputs(
            turn.request_round,
            turn.drive_seq,
            result,
        )],
    )
    .map_err(ActorError::Append)
}

/// 重试开出的**新支在这里登记**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 为什么是这一格而不是唤醒那一处：`turn.branch` 的 `into` 是**新支的 Turn 根
/// id**，而那个 id 到本幕的根定下才存在（一幕一个根，幕首是它唯一的创建点）。
/// `retry_of` 由 `RetrySubtree` 寄存、`take()` 即消费，幕间续写的第二幕
/// 因此不会再开一支。
pub(crate) fn land_retry_branch(
    store: &dyn Store<Event = Event>,
    ctx: &dyn PluginInvokeRequest,
    turn: &mut super::chat_loop::state::TurnState,
    root_id: &str,
) -> Result<(), ActorError> {
    let Some(of) = turn.retry_of.take() else {
        return Ok(());
    };
    let seq = store
        .append(
            symbio_core::Event::pending(
                format!("branch-{root_id}"),
                symbio_plugin_api::events::EVENT_TURN_BRANCH,
                Entity::Turn,
                Verb::Asserted,
                turn.turn_no,
                symbio_core::request_principal(ctx),
            )
            .with_payload(serde_json::json!({ "of": of, "into": root_id }))
            .with_produced_by(turn.user_seq),
        )
        .map_err(ActorError::Append)?;
    symbio_core::plugin_info!(
        "session",
        "[Branch] 新支 {} 挂到 {} 之下（seq={}）",
        root_id,
        of,
        seq.value()
    );
    // 收尾时按**这一格**补发组内的标注帧（`finish_turn` 从本幕的事实切片读回
    // `into`）：新支的根 id 不再在草稿上另存一份。
    Ok(())
}

/// 账本上的这一格是否让**本轮收工**（D-123 决策 5 的收束判据）。
///
/// 三种形态，各对应「本轮不再往下走」的一个理由：
/// - 收束格 / 兜底格（[`is_stop_fact`] 的第一臂）；
/// - **收束闸**被拒的否定事实——`authz.denied` **不带 `port`** 的那一条。带 `port`
///   的是**端口面**的授权拒绝（`tool_dispatch::denied_event`），它说的是「那一笔工具
///   没放行」，拒绝载荷回灌给模型后**轮还在走**；拿 core 的停机尺（C6 问「这一轮为什么
///   停」，两种拒绝都要答）直接当收工判据，会把 auto 幕的一次拒批读成本轮结束
///   （判据 = e2e `t78` 的 auto 幕）。
/// - 收束于**等待用户**那一格进展（`status: "pending"`）：它不是停机格（本轮尚未了结，
///   收束格留给恢复幕），但它是本轮的最后一步——继续选债就会再跑一次模型往返，
///   把「等你回答」跑成「又答了一轮」。字面口径与这里的差异记在 D-123。
fn ends_the_drive(e: &Event, turn: u64) -> bool {
    if e.turn != turn {
        return false;
    }
    if is_stop_fact(super::control_decl::guard_vocab(), e) {
        return e.kind != EVENT_AUTHZ_DENIED || e.payload.get("port").is_none();
    }
    e.kind == EVENT_CHAT_ASSISTANT_PROGRESS
        && e.payload.get("status").and_then(|v| v.as_str()) == Some("pending")
}

#[cfg(test)]
#[path = "turn_exec.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::current_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::current_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::finalize_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::finalize_reasoning_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::on_delta: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::on_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::send: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::close: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-loop/src/turn_exec.rs::pop: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
