//! v2 执行器——轮次经 v2 运行器**原生执行**（唯一执行路径，ADR-051）。
//!
//! 事实与呈现同源（网格不漏轮也不双记）：事实在生成路径上**直接入格**——轮次事实
//! （`u-{turn}` 开口 / 收束格）由 v2 运行器落，派生事实由本模块轮末补（见下）；
//! prompt 从转写出（同一事实源，ADR-044 同族），流式增量经 [`UiBridge`]
//! 回到事件出口（用户看到的仍是逐字上屏的对话面）。
//!
//! 覆盖面（诚实划界）：**无工具轮与工具轮**。工具分发走工具执行层
//! （`process_tool_calls_async`：超长结果存档、生命周期钩子、审批与恢复都在里面），
//! 由 [`super::tool_dispatch::SessionDispatchPort`] 经插件本地的
//! [`super::tool_dispatch::ToolDispatch`] 三动作契约接入——**分发是插件侧的实现细节，
//! 不是第二条执行链**。
//!
//! ## 反射档（S11 快路）：第三条路，也是唯一不调模型的一条
//!
//! 开 `skill_fast_path` 且本轮发言**逐字命中**一条已编译技能时，本轮改走
//! [`TurnRunner::run_reflex`]——以技能正文收束，**一次模型调用都不发生**。
//! 它不是"又一条执行链"：产物落的是**同一轮的三格**（`u-{turn}` / `turn.dispatch` /
//! `f-{turn}`），收束后仍走同一段 `record_learning`，收口（步骤 5–7）也完全共享；
//! 变的只是**生成那一格**由谁产出（反射档没有 `llm` 形参 ⇒ "反射档调模型"写不出来）。
//!
//! 两条边界，缺一条都会静默走错：
//! - **只对新开轮**：续写轮不新开用户格（调用方直给原轮的 `user_seq`），而反射档的
//!   落格形状就是新开一轮 ⇒ 走它会撞幂等键（`Duplicate`），既完不成收束也把本轮变成错误；
//! - **开关默认关**：关着时候选集恒空，本轮照常走 `run_with_tools`。
//!
//! 反射档把 `actor.budget_ms` 按档位派生（`LatencyTier::Reflex.budget_ms()`），
//! 该值随主体走、并由 `execute_turn` 以**数值**写入 `turn.dispatch` 载荷的 `budget_ms`
//! （T-082 / T-225）——`invariants::declared_budget_ms` 优先读它，`ActorSpec.budget_ms`
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
//! - [`super::derived_facts::record_derived`]：承诺（`commitment_events`，S08 §3）/
//!   任务表（`task_events::write`，S7 步 16–18）/ 熔断（`CircuitBreaker::break_event`，
//!   S8 步 20）——三者的数据来源都在工具执行层（`Delegation` / `TaskDeclaration` /
//!   熔断理由），由 [`super::tool_dispatch::SessionDispatchPort`] 经 `take_derived` 交回。
//!
//! ## 写侧授权闸（判在运行器）
//!
//! **写侧授权闸**（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）判的是「主体此刻
//! 能否写本轮的收束格」（`reply.first` / `reply.append`），由运行器在落收束格**之前**
//! 判（`turn_runner::closure_granted`，谓词 `can_reply` 只定义一次）。拒绝 ⇒ 收束格
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

use crate::symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use crate::symbio_core::{
    llm_short_id, run_ops, ActorSpec, AppendError, CapabilityMeta, ConsolidateRequest, DeltaSink,
    Entity, Event, ExecAbortSignal, ExecEventSink, FullModel, LatencyTier, LlmAdapter,
    ModelProvider, Plugin, PluginError, ProviderLlmAdapter, Seq, SilentDeltas, Store, TokenIssuer,
    TurnOutput, TurnToolCallInfo, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};

// 压缩入格与寄存补落都是**会话动作**（事实源归档不在 core `Store` 面内，写方必须
// 留在持令牌的 session）；本文件只剩轮内调用点，经 [`LoopSession`] 的端口动作发起。
use super::turn_runner::{TurnInput, TurnOutcome, TurnResume, TurnRunner, TurnSessionBegin};
// 运行器一族**已下沉到本插件**（2026-10-09，见 `turn_runner.rs` 的头注）：它们不是 core 的
// 契约面，生产消费方只有本插件，按 README §4 四问第 1 问该住这里。
use super::turn_session::LoopSession;

// —— 阶段拆分（T-179 · B2，PL-015 §5 动作 B2）——
// `execute_turn` 的四个处理阶段各自成文件；再导出保持 `turn_exec::*` 的既有路径不变
// （消费方与本文件测试都不必改引用）。阶段体是逐字搬移，本文件只留调度。
mod assemble;
mod call;
mod outcome;
mod persist;

pub(crate) use self::assemble::{assemble_turn, Assembled};
// 续写幕的开幕方（`chat_loop::drive`）要用同一个「已开未收束」判据：
// 这里的收束口径（只认 final / fallback）与 C4（兼认产物）刻意不同，见函数文档。
pub(crate) use self::assemble::last_open_turn;
pub(crate) use self::call::{call_model, Running};
pub(crate) use self::outcome::{classify_turn, Classified, ExecTurnResult, Settled, TurnExecExit};
pub(crate) use self::persist::{execute_final_reply_turn, persist_turn};

/// 恢复产生的工具交换（审批 / 问答恢复后，由恢复路径交回）。
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
    /// **被恢复的 ToolCall 节点 id**（= 恢复请求的 `target_id`）。
    ///
    /// 续写轮落 `artifact.added` 时要把它记进载荷的 `tool_call_id`：读路径走 WAL
    /// 重建后，「结果节点挂在哪个调用之下」只能靠这条事实复刻。缺了它，重建会
    /// 按事件 id 合成一个新调用节点 ⇒ 原调用成了**孤儿**（有请求无响应，e2e t27 抓过）。
    pub call_id: String,
    /// **被恢复轮次的 Turn 根节点 id**（原调用节点的 `parent_id`）。
    ///
    /// 同上：重建把结果节点挂在 `tool_id` 之下，而调用节点自身要落在**原来那一轮**
    /// 的 Turn 根上——否则同一轮会长出第二个 Turn 根。
    pub turn_id: String,
    /// 结果**子节点**的节点 id（恢复方分配，`resume.rs` 步 7 的 `new_child_id`）。
    ///
    /// `Carrier::Runner` 的语义是「这条节点由运行器承载、不落出生事实」；承载它的
    /// 事实就是本次 `artifact.added`——载荷必须复用**同一个 id**，否则实时面与重建
    /// 面是两条不同的节点（前端按 id 合并，换 id 即多出一条）。
    pub result_id: String,
}

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
    /// 给这一帧的节点定形状：正文是 `Body`，推理是 `Notice`（折叠叶，[ADR-070 §决策 3]
    /// [T-155]——reasoning 与 compression 同格，前端渲染二者一样，业务区分只在 `style_tag`）。
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
        self.tx.close();
    }
}

impl UiBridge {
    /// 当前正文节点 `(id, 累积正文)`（未定格的最后一轮正文；本轮无正文 ⇒ `None`）。
    ///
    /// 累积正文的消费点有两个：`chat_loop` 的 `finalize_assistant_turn`（收口末轮）
    /// 与中止出口（半截正文入格，见 [`execute_turn`]）。
    pub(crate) fn current_node(&self) -> Option<(String, String)> {
        self.node.lock().unwrap().clone()
    }

    /// 当前推理节点 `(id, 累积正文)`（未定格的最后一轮推理；本轮无推理 ⇒ `None`）。
    ///
    /// 与 [`Self::current_node`] 的分工：正文的末轮由 `chat_loop` 的
    /// `finalize_assistant_turn` 收口（那里拿着 `TurnOutput.text`），推理那侧同理
    /// 但正文得**从这里取**——`TurnOutput` 的 `reasoning` 字段正是由本函数回填的
    /// （见 [`execute_turn`] 轮末）。
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
            crate::plugin_warn!(
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
            Box<
                dyn std::future::Future<Output = Option<crate::symbio_core::DrainedSupplement>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// 轮边界回调（缺口 4）：工具循环每跑完一轮问一次「要不要说点什么」。
///
/// 由 `chat_loop` 提供——它有编排器与 compose，判定与措辞都在那边；本函数只负责
/// **把它交回来的草稿落格**（这里开着 `EventWalStore`）。类型定义与理由见
/// `round_hook.rs`。
pub(crate) use super::round_hook::RoundHookFn;

pub(crate) struct ExecTurn<'a> {
    /// 会话窄句柄（交付包 + 动作面端口；不持有会话存储实现）。
    pub session: &'a LoopSession,
    /// 本轮生效的模型服务（core trait 的 trait object）。
    pub provider: Arc<dyn ModelProvider>,
    /// 插件宿主（工具能力经它路由；无工具轮为 `None`）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本轮请求上下文（工具路由与能力注册表都在它上面）。
    pub ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    /// 会话级系统提示词（取值点 = `prepare_turn_inputs`）。
    pub system_prompt: &'a str,
    /// 本轮中止信号（同样由 chat_loop 传入）。
    pub abort: ExecAbortSignal,
    /// 本轮 Turn 根节点 id（UI 帧挂载的父节点）。
    pub root_id: &'a str,
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
    /// 续写锚点：`Some` ⇒ 本轮续写**已开未收束**的轮次（审批 / 问答恢复），
    /// `None` ⇒ 新开一轮。见 [`ResumedTool`] 与 core 的 `TurnResume`。
    pub resume: Option<ResumedTool>,
    /// 本轮开头召回的长期记忆视图（S5 步 12）——轮末由
    /// [`super::derived_facts::record_learning`] 落一条 `memory.recalled`。
    ///
    /// 它**不是**轮次事实（轮次事实由运行器原生入格），而是派生事实——写方是
    /// [`super::derived_facts::record_learning`]（见该函数文档）。
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
    /// 轮边界汇报（缺口 4）：`None` = 不汇报（`progress_enabled = false`，或未挂 compose）。
    ///
    /// 与 [`Self::supplements`] **共用同一个挂点**（`TurnInput::inject`）：core 只知道
    /// 「一轮跑完了，问一次」。分成两个口就得让 core 记两件它不需要知道的事。
    pub on_round: Option<RoundHookFn>,
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
    pub recalled: Option<&'a crate::symbio_core::RecallView>,
    /// 本轮技能路由判定（S11 步 22）：`(skill_id, fallback)` 逐条——轮末落
    /// `memory.recalled` 供 `calibration` 归并，是「回退会发生」的唯一数据源。
    pub skill_obs: &'a [(String, bool)],
    /// 本轮**可用**的技能集（S11 快路的候选）：本轮发言逐字命中其中一条 ⇒ 本轮
    /// 走**反射档**（[`TurnRunner::run_reflex`]），一次模型调用都不发生。
    ///
    /// 空表是**常态**（没开 `skill_fast_path` / 没编过技能）——
    /// 那时本轮照常走 `run_with_tools`。填充点唯一：
    /// `chat_loop/inputs.rs` 的 `fast_armed` 分支（与观测同一次 `route`）。
    pub skill_hits: &'a [super::skill_events::SkillLlmHit],
    /// **请求级前缀**：请求视图层 `build_request_view` 置顶的三段——长期记忆召回 /
    /// 就绪任务集 / 委派者真源——在这里拼成一段文本，排在本轮对话**之前**
    /// （见 [`TurnInput::prefix`](super::turn_runner::TurnInput::prefix)）。
    ///
    /// ## 为什么必须显式带进来
    ///
    /// 会话轮的 prompt 是**一条** user 消息（`ProviderLlmAdapter::generate_turn`
    /// 把渲染结果整段发出），而那三段是**独立的消息**。不显式带进来它们就会
    /// **静默**从模型眼前消失——事实照样入格、`session/stats` 照样有数，只有模型
    /// 看不见（实测 t29 / t35 报「记忆段没注入」，而注入逻辑一行没改）。
    ///
    /// ## `None` 是常态
    ///
    /// 三段都空的情形：首轮没有记忆、没有就绪任务、判定不出委派者。取法见
    /// `chat_loop` 的 `request_view_prefix`。
    pub prefix: Option<String>,
    /// **本轮之前发生过压缩** ⇒ 压缩后的整份消息列表（S2.7-b2）。
    ///
    /// 由 `chat_loop` 从 [`SessionContext::consolidated`] 带来。落格动作在
    /// [`execute_turn`] 里做——经会话端口的 `consolidate` 动作（持令牌的写方在
    /// session；压缩现场不持令牌，就地落格会拿到 `NotTheWriter`，而那是静默的）。
    ///
    /// `None` = 本轮没压过（常态）。
    ///
    /// [`SessionContext::consolidated`]: super::chat_loop::SessionContext::consolidated
    pub consolidation: Option<&'a [ChatNode]>,
    /// 本轮**已执行的一次性阶段**（`turn.stage` 草稿，ADR-107 判据 2）。
    ///
    /// 由 `chat_loop` 在阶段执行点产出（判决 / 召回 / 压缩），落格在运行器——
    /// 用户格之后（I2 要求断言类事件带溯源）。两档都要落：深度档经
    /// [`super::turn_runner::TurnInput::stages`]，反射档经 `run_reflex` 的入参。
    pub stages: Vec<super::stage_events::StageFact>,
    /// **对话面开场白正文**（T-056）：`Escalate` 首响，本轮随后照进工具循环。
    ///
    /// 由 `chat_loop::apply_verdict` 产出（`VerdictEffect::Spoke` 且理由码非
    /// `progress`——汇报走 `on_round`/`EVENT_ASSISTANT_REPORTED`）。落格在
    /// [`execute_turn`] 里做（本插件唯一持写者令牌处），运行器据它落
    /// `chat.assistant.opening` 格。`None` = 本轮没有开场白（常态）。
    pub opening_text: Option<&'a str>,
    /// 开场白节点身份与标记（仅 [`Self::opening_text`] 有值时消费）。
    pub opening_dialog: Option<super::turn_runner::DialogFacts>,
    /// **本轮建树策略**（`SessionConfig::presentation_tree` 的请求级快照）。
    ///
    /// 与 v1 流式（`ChatOrchestrator::presentation_tree`）**同一个取值快照**——一次
    /// 请求里 v1、v2 必须按同一档建树，否则 live↔存储↔重建三格错位（树形状锁步）。
    /// 由 `chat_loop` 从编排器带入，本函数不读配置（配置在请求生命周期内不变，
    /// 取一次即够）。
    pub presentation_tree: crate::symbio_core::node_build::PresentationTree,
}

/// 驱动一轮：每步读事实切片 → `loop_rows::next` 选最小可还债 → 跑**一个执行体
/// 动作**，直到收束。
///
/// T-227 步 5 把循环从 `turn_runner` 移到这里；T-233 把「执行器自己知道
/// 下一步」换成「引擎从行表读」：`decl::ring::drive` 是生产入口，派发器不再
/// 手写 `begin` / `step` 的推进条件，也不按行名 / 端口名分支（ADR-118 决策 1–2）。
///
/// 过渡态（ADR-118 后果 2 进行中）：执行体注册表尚未按 `by` 一一接线，`policy.*`
/// 与三笔工具债仍在内环里完成；派发器按**自己持有的草稿**选动作——有寄存的工具批
/// 就跑 [`TurnSession::tools`]，没有就跑 [`TurnSession::model`]。因此
/// `turn_runner` 只回答「一个动作发生了什么」，不再在一个 `step` 里同时跑模型与工具。
/// 停机判据（64 步 / 无进展 / 中止 / 租约）留在派发器，不随选债口搬家。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn drive_steps<S>(
    store: &S,
    llm: &dyn LlmAdapter,
    tok: &FullModel,
    input: TurnInput,
    sink: Arc<dyn DeltaSink>,
    tools: &[CapabilityMeta],
    dispatch: Option<&dyn super::tool_dispatch::ToolDispatch>,
) -> Result<TurnOutcome, AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let step_sink = sink.clone();
    // 续写幕旁路（**不是兼容层**，见下方 `next.is_none` 处的理由）：`turn_head` 是
    // 全局最新一幕的表头，恢复一条已被后续轮次归档的旧轮时，表头已指向新轮。
    let resumed = input.resume.is_some();
    let mut session = match TurnRunner::begin(store, llm, input, sink).await? {
        TurnSessionBegin::Done(outcome) => return Ok(outcome),
        TurnSessionBegin::Run(session) => session,
    };
    loop {
        // ADR-118 决策 2：派发器读切片 → 选最小可还债 → 按 `by` 查执行体注册表 →
        // 跑**一个**执行体动作，然后重新读切片。不按行名 / 端口名 / 业务语义分支。
        let facts = store.range(Seq::new(0));
        let next = super::loop_rows::next(
            &facts,
            session.turn(),
            crate::symbio_core::clock_now_ms(),
            Executor::accepts,
        );
        if next.is_none() && !resumed {
            // 没有可还的债：本幕要么已经收束，要么表装载失败 / 只剩 blocked 行。
            // 不拿一个假成功返回——落 `exhausted{no_progress}` 让它在事实面显形。
            //
            // 续写幕走旁路：行表只有**一个**全局表头（`turn_head` 取最新一轮），
            // 恢复一段被后续轮次归档的旧轮时它指向新轮，旧轮的 `execute-turn`
            // 在表里看不到。这限制在表侧，堵在这里只会让恢复永远 `no_progress`
            // （用例 `an_open_turn_archived_by_the_next_turn_is_still_the_resume_anchor`）。
            return session.exhausted(store, llm, "no_progress").await;
        }
        // 执行体：**粗端口期内环**（PL-016 §7.2）——`execute-turn` 在整轮收束前恒欠，
        // 每步都落在它上面；细端口债（`dispatch-model` / `open-tools` /
        // `authorize-tool` / `dispatch-tools`）由这个执行体**内部**写，
        // 不另起第二调度器（否则两批之间会有两个调度器抢同一份事实）。
        // 动作由派发器持有的草稿决定（模型往返 ↔ 工具批次，ADR-118 决策 4）。
        //
        // 按注册表筛过的选债面：注册表还没覆盖到的 `by`（`policy.*` 与三笔工具债，
        // 粗端口期都由整轮执行体内部完成）不会被选中，不能让它把整轮顶成
        // `no_progress`。
        let executor = match &next {
            Some(next) => {
                crate::plugin_debug!(
                    "session",
                    "[Loop] 派发 {}（执行体 {}；债 {}）",
                    next.row,
                    next.by,
                    next.key
                );
                // 选债面已按同一个注册表筛过；这里再查一次只为消灭「选中但没有
                // 执行体」的静默状态——查不到就按 no_progress 显形，不猜动作。
                let Some(executor) = Executor::for_by(next.by) else {
                    crate::plugin_warn!(
                        "session",
                        "[Loop] 行表与执行体注册表脱节（选中 `by={}` 无执行体），按 no_progress 收束",
                        next.by
                    );
                    return session.exhausted(store, llm, "no_progress").await;
                };
                executor
            }
            // 续写幕旁路：行表看不见被归档的旧轮（见上方 `is_none && !resumed`），
            // 按整轮执行体继续跑——这是旁路唯一的作用点。
            None => Executor::Execute,
        };
        // 草稿归派发器所有（ADR-118 决策 4）：有寄存的工具批就派发它，否则跑模型。
        let has_batch = session.has_pending_batch();
        if !has_batch && session.count_model_step() {
            crate::plugin_warn!(
                "actors",
                "[v2] 轮内步数超过上限（turn={}），收束为 exhausted（step_limit）",
                session.turn()
            );
            return session.exhausted(store, llm, "step_limit").await;
        }
        let head = facts.len();
        let outcome = match executor {
            // 粗端口期唯一执行体：按草稿跑一次模型往返或一次工具批。
            Executor::Execute => {
                if has_batch {
                    session.tools(store, llm, dispatch).await?
                } else {
                    session
                        .model(
                            store,
                            llm,
                            tok,
                            step_sink.clone(),
                            tools,
                            dispatch.is_some(),
                        )
                        .await?
                }
            }
        };
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        // ADR-118 决策 7：每步之后比较日志头。执行体返回空且事实没长
        // ⇒ 环不会前进，落 `exhausted{no_progress}` 而不是空转到步数上界。
        if store.range(Seq::new(0)).len() == head {
            return session.exhausted(store, llm, "no_progress").await;
        }
    }
}

/// 派发器的执行体注册表（ADR-118 决策 2）：`Debt.decl.by` → 执行体。
///
/// **粗端口期只有一档**（ADR-118 决策 3）：`execute-turn` 在整轮收束前恒欠，
/// 工具三笔债由它在内部按三个动作写。选债时按注册表筛：还没接线的 `by`
/// （`policy.*` 与三笔工具债）在粗端口期都由整轮执行体内部完成，它们不参与
/// 派发器选债——否则一笔内环债会把整轮顶成 `no_progress`。
/// 其余 `by` 的执行体随对方行在 PL-017 的细端口里变得可选中再接。
#[derive(Clone, Copy)]
enum Executor {
    /// 本轮的执行体：按持有草稿跑一次模型往返或一次工具批。
    Execute,
}

impl Executor {
    fn for_by(by: &str) -> Option<Self> {
        match by {
            "execute" => Some(Self::Execute),
            _ => None,
        }
    }

    /// 选债面用的谓词：`by` 有没有执行体。
    fn accepts(by: &str) -> bool {
        Self::for_by(by).is_some()
    }
}

/// 一轮的执行：装配（阶段①）→ 调用（阶段②）→ 结局归类（阶段③）→ 落格（阶段④）。
///
/// T-179 · B2：原函数体按处理阶段逐字拆入 `turn_exec/{assemble,call,outcome,persist}.rs`，
/// 本函数只留阶段间调度——调度点即原阶段边界，副作用顺序与事件批次序不变。
pub(crate) async fn execute_turn(req: ExecTurn<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    let assembled = assemble_turn(req).await?;
    let running = call_model(assembled).await?;
    match classify_turn(running).await? {
        Classified::Finished(v) => v,
        Classified::Settled(settled) => persist_turn(settled),
    }
}
#[cfg(test)]
#[path = "turn_exec.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/plugins/loop/turn_exec.rs::current_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::current_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::finalize_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::finalize_reasoning_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::on_delta: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::on_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::send: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::close: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/loop/turn_exec.rs::pop: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
