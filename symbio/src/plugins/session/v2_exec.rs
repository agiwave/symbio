//! v2 执行器——轮次经 v2 运行器**原生执行**（唯一执行路径，ADR-051）。
//!
//! 事实与呈现同源（网格不漏轮也不双记）：事实在生成路径上**直接入格**——轮次事实
//! （`u-{turn}` 开口 / 收束格）由 v2 运行器落，派生事实由本模块轮末补（见下）；
//! prompt 从转写出（同一事实源，ADR-044 同族），流式增量经 [`UiBridge`]
//! 回到事件出口（用户看到的仍是逐字上屏的对话面）。
//!
//! 覆盖面（诚实划界）：**无工具轮与工具轮**。工具分发走工具执行层
//! （`process_tool_calls_async`：超长结果存档、生命周期钩子、审批与恢复都在里面），
//! 由 [`super::v2_tools::SessionDispatchPort`] 经 core 的 [`crate::symbio_core::DispatchPort`]
//! 契约接入——**分发是插件侧的实现细节，不是第二条执行链**。
//!
//! ## 反射档（S11 快路）：第三条路，也是唯一不调模型的一条
//!
//! 开 `skill_fast_path` 且本轮发言**逐字命中**一条已编译技能时，本轮改走
//! [`TurnRunner::run_reflex`]——以技能正文收束，**一次模型调用都不发生**。
//! 它不是"又一条执行链"：产物落的是**同一对格子**（`u-{turn}` / `f-{turn}`），
//! 收束后仍走同一段 `record_learning`，收口（步骤 5–7）也完全共享；变的只是
//! **生成那一格**由谁产出（反射档没有 `llm` 形参 ⇒ "反射档调模型"写不出来）。
//!
//! 两条边界，缺一条都会静默走错：
//! - **只对新开轮**：续写轮不新开用户格（复用 `resume.user_seq`），而反射档的落格形状
//!   就是新开一轮 ⇒ 走它会撞幂等键（`Duplicate`），既完不成收束也把本轮变成错误；
//! - **开关默认关**：关着时候选集恒空，本轮照常走 `run_with_tools`。
//!
//! 反射档把 `actor.budget_ms` 按档位派生（`LatencyTier::Reflex.budget_ms()`），
//! 该值随主体走、并由 `execute_turn` 以**数值**写入开轮格载荷的 `budget_ms`
//! （T-082 批①）——`invariants::declared_budget_ms` 优先读它，`ActorSpec.budget_ms`
//! 于是有了真读方（此前是"算了却看不见"）。缺该数值的历史事件回退读 `tier`
//! 字符串再映射，两条路径读的是**同一条事实的两种形态**，见 `declared_budget_ms` 文档。
//!
//! ## 收束派生事实（本模块已覆盖）
//!
//! 轮次事实（`u-{turn}` 开口 / 收束格）由运行器原生入格；**派生事实不是轮次事实**，
//! 而是本轮的派生副作用——运行器一处都不写，写方只有轮末这两个函数
//! （溯源锚 = 这里原生写的 `u-{turn}` 格）：
//!
//! - [`super::v2_facts::record_learning`]：步 11 编码 / 步 12 检索锚 / 步 13 巩固 /
//!   步 22 技能观测与编译；
//! - [`super::v2_facts::record_derived`]：承诺（`commitment_events`，S08 §3）/
//!   任务表（`v2_tasks::write`，S7 步 16–18）/ 熔断（`CircuitBreaker::break_event`，
//!   S8 步 20）——三者的数据来源都在工具执行层（`Delegation` / `TaskDeclaration` /
//!   熔断理由），由 [`super::v2_tools::SessionDispatchPort`] 经 `take_derived` 交回。
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
//! `status: "aborted"` + `terminated: { reason, at }`），本函数上抛
//! `Aborted { partial }`——chat_loop 走独立出口，会话结局 `aborted`
//! （**不是** `failed`），半截产出随 `partial` 入格。
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
    llm_short_id, ActorSpec, CapabilityMeta, DeltaSink, DispatchPort, Entity, Event, EventWalStore,
    ExecAbortSignal, ExecEventSink, LatencyTier, ModelProvider, Plugin, PluginError,
    ProviderLlmAdapter, Seq, SilentDeltas, Store, TokenIssuer, TurnOutput, TurnToolCallInfo, Verb,
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;
// 运行器一族**已下沉到本插件**（2026-10-09，见 `turn_runner.rs` 的头注）：它们不是 core 的
// 契约面，生产消费方只有本插件，按 README §4 四问第 1 问该住这里。
use super::turn_runner::{TurnInput, TurnResume, TurnRunner};

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
    /// 分发方（[`super::v2_tools::SessionDispatchPort`]）既走通道（定格正文）
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
            // 它在本函数上真实发生过一次（见 `v2_exec.test.rs` 里 `drained` 的注释）。
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
    /// `v2_exec.test.rs` 调它们。生产构建里它们无人调用 ⇒ `clippy -D warnings`
    /// 判死代码。标 cfg(test) 而不是加 `dead-code-allow` 豁免——豁免是给「生产里
    /// 确实需要、只是暂时没人调」的东西用的，而这里的需求本来就不在生产侧。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.lock().unwrap().q.len()
    }

    /// 软超容累计（恒应为 0）。
    /// ⚠️ `#[cfg(test)]`：这两个是**判据用的观察口**，只有
    /// `v2_exec.test.rs` 调它们。生产构建里它们无人调用 ⇒ `clippy -D warnings`
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
/// `pub(crate)`：工具分发方（[`super::v2_tools::SessionDispatchPort`]）要经
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
        // 而调用方（`v2_tools::dispatch` 的 ①）紧接着就把工具节点**直接**写出口——
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
/// 由 `chat_loop` 从 `SupplementDrain` 适配而来（它要插件宿主 + 会话状态，那个
/// 类型不进 `V2Turn`）。测试直接给闭包，于是能造「只返回一次」这种形状。
pub(crate) type SupplementFn =
    Arc<dyn Fn() -> Option<super::chat_loop::state::DrainedSupplement> + Send + Sync>;

/// 轮边界回调（缺口 4）：工具循环每跑完一轮问一次「要不要说点什么」。
///
/// 由 `chat_loop` 提供——它有编排器与 compose，判定与措辞都在那边；本函数只负责
/// **把它交回来的草稿落格**（这里开着 `EventWalStore`）。类型定义与理由见
/// `round_hook.rs`。
pub(crate) use super::round_hook::RoundHookFn;

pub(crate) struct V2Turn<'a> {
    /// 会话（v2 WAL 目录与会话配置的持有者）。
    pub session: &'a PersistentChatSession,
    /// 本轮生效的模型服务（core trait 的 trait object）。
    pub provider: Arc<dyn ModelProvider>,
    /// 插件宿主（工具能力经它路由；无工具轮为 `None`）。
    pub parent: Option<Arc<dyn Plugin>>,
    /// 本插件目录（工具结果存档落在这里）。
    pub session_dir: crate::symbio_core::PluginDir,
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
    /// 本轮用户发言（事实入格与 prompt 的同一来源）。
    pub user_text: &'a str,
    /// 本轮用户消息节点 id（落库镜像里那条用户消息的 id）——收束格 / 进展格的
    /// 载荷记它，重建侧据此把本轮锚回用户发言。id 由 chat_loop 分配（消息镜像的
    /// 写方），这里只读不生成。
    pub user_node_id: Option<String>,
    /// 本轮用户消息节点的 `meta`（`None` = 无 meta）。折进的合并补充带
    /// `meta.supplement*`——它是节点形状的一部分，必须随用户格事实入格（见
    /// [`super::turn_runner::TurnInput::user_meta`]）。
    pub user_meta: Option<serde_json::Value>,
    /// 对话窗口（最近轮数，含当前轮）。
    pub window_turns: u64,
    /// 本轮装配进模型的工具清单（空 = 无工具轮）。
    pub tools: &'a [CapabilityMeta],
    /// 续写锚点：`Some` ⇒ 本轮续写**已开未收束**的轮次（审批 / 问答恢复），
    /// `None` ⇒ 新开一轮。见 [`ResumedTool`] 与 core 的 `TurnResume`。
    pub resume: Option<ResumedTool>,
    /// 本轮开头召回的长期记忆视图（S5 步 12）——轮末由
    /// [`super::v2_facts::record_learning`] 落一条 `memory.recalled`。
    ///
    /// 它**不是**轮次事实（轮次事实由运行器原生入格），而是派生事实——写方是
    /// [`super::v2_facts::record_learning`]（见该函数文档）。
    /// 补充抽干口（缺口 3）：工具循环每跑完一轮问一次「用户有没有中途插话」。
    /// 有就折进**本轮**——落一格 `turn.supplemented` 事实 + 进下一次请求。
    ///
    /// **闭包而不是具体类型**：抽干的实现是 `SupplementDrain`（要插件宿主 + 会话
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
    pub skill_hits: &'a [super::v2_skills::SkillLlmHit],
    /// **请求级前缀**：请求视图层 `build_request_view` 置顶的三段——长期记忆召回 /
    /// 就绪任务集 / 委派者真源——在这里拼成一段文本，排在本轮对话**之前**
    /// （见 [`TurnInput::prefix`](crate::plugins::session::turn_runner::TurnInput::prefix)）。
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
    /// [`execute_turn`] 里做——那是本插件**唯一**持写者令牌的时刻（见
    /// `consolidate_into_wal` 的文档：压缩现场落格会拿到 `NotTheWriter`，而那是
    /// 静默的）。
    ///
    /// `None` = 本轮没压过（常态）。
    ///
    /// [`SessionContext::consolidated`]: super::chat_loop::SessionContext::consolidated
    pub consolidation: Option<&'a [ChatNode]>,
    /// **对话面开场白正文**（T-056）：`Escalate` 首响，本轮随后照进工具循环。
    ///
    /// 由 `chat_loop::apply_verdict` 产出（`VerdictEffect::Spoke` 且理由码非
    /// `progress`——汇报走 `on_round`/`EVENT_ASSISTANT_REPORTED`）。落格在
    /// [`execute_turn`] 里做（本插件唯一持写者令牌处），运行器据它落
    /// `chat.assistant.opening` 格。`None` = 本轮没有开场白（常态）。
    pub opening_text: Option<&'a str>,
    /// 开场白节点身份与标记（仅 [`Self::opening_text`] 有值时消费）。
    pub opening_dialog: Option<crate::plugins::session::turn_runner::DialogFacts>,
    /// **本轮建树策略**（`SessionConfig::presentation_tree` 的请求级快照）。
    ///
    /// 与 v1 流式（`ChatOrchestrator::presentation_tree`）**同一个取值快照**——一次
    /// 请求里 v1、v2 必须按同一档建树，否则 live↔存储↔重建三格错位（树形状锁步）。
    /// 由 `chat_loop` 从编排器带入，本函数不读配置（配置在请求生命周期内不变，
    /// 取一次即够）。
    pub presentation_tree: super::config::PresentationTree,
}

/// 一轮 v2 原生产物的**全部出口**。
pub(crate) struct V2TurnResult {
    /// 收口用产物（正文 / 节点 id）——`TurnOutput` 同形，收口步骤 5–7 只认这一份结构。
    pub output: TurnOutput,
    /// 本轮 v2 原生**额外产生**的消息（工具轮：助手中途正文 + ToolCall + 工具结果），
    /// 按产生顺序。chat_loop 把它们并入 `context.messages`——落库权威是那份
    /// 内存镜像（`persist_messages` 只写它的尾部增量），不并入就只看得见不落库。
    pub messages: Vec<ChatNode>,
}

/// 一轮的执行：事实原生入格 + 生成（含工具循环）+ 流式回 UI。
/// 抽干结果 → **落格 + 构造注入消息**（缺口 3 的全部逻辑，闭包之外）。
///
/// 为什么抽出来：闭包在 P3c 里已经 90 行，且一半是「抽干 / 落格 / 造节点」的直线代码。
/// 缺口 4 要往同一个闭包里再加一段汇报逻辑，不抽出来的话那个闭包会奔到 200 行——
/// 而它的可读性靠的正是「一眼看到这一轮边界上依次发生了什么」。
///
/// 返回**空 vec** = 本轮无可注入（没有补充 / 正文为空 / 拿不到溯源锚点）。三种情形
/// 都**不消耗任何配额**、不留半截状态，与 `apply_verdict` 拿不到措辞时同一条方向。
#[allow(clippy::too_many_arguments)] // 抽出来之后参数就是它的全部依赖，多一个都该重新想
fn supplement_into_request(
    batch: Option<super::chat_loop::state::DrainedSupplement>,
    anchor: Option<u64>,
    store: &crate::symbio_core::EventWalStore,
    actor: &ActorSpec,
    turn_no: u64,
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatNode>>>,
    no_counter: &std::sync::atomic::AtomicU64,
) -> Vec<crate::symbio_core::PromptMessage> {
    let Some(drained) = batch else {
        return Vec::new();
    };
    let (text, count, ids) = (drained.text, drained.count, drained.ids);
    if text.trim().is_empty() {
        return Vec::new();
    }
    // **落格**：成为一格事实（`turn × asserted`）。不落格的话它只活在这一次请求里，
    // 下一轮又消失了——而「用户说过的话没有变成事实」正是缺口 3 本身。
    //
    // 事件 id 带序号（`s-{turn}-{no}`）：一次工具循环里可能折进多条，不带序号会撞
    // 幂等键（`Duplicate`）。
    //
    // 拿不到溯源锚点 ⇒ **不落也不注入**：I2 要求断言类事件带溯源，而无溯源的断言
    // 事件比不落更难查。
    let Some(anchor) = anchor else {
        crate::plugin_warn!(
            "session",
            "[Turn] 补充无溯源锚点，本轮丢弃（见本函数「拿不到溯源锚点」处）"
        );
        return Vec::new();
    };
    let no = no_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = store.append(
        crate::symbio_core::Event::pending(
            format!("s-{turn_no}-{no}"),
            crate::symbio_core::EVENT_TURN_SUPPLEMENTED,
            crate::symbio_core::Entity::Turn,
            crate::symbio_core::Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(anchor)
        // 载荷带上**节点身份**（T-052 的原则：节点身份是事实的一部分）：重建按 `id`
        // 复刻这条用户消息的身份、按 `ids` 复刻 `meta.supplement_ids`。缺了它们，
        // 重建只能合成 id（每次重开都换一批），且合并标记里的原始条目 id 无处可寻
        // ——而「合并后原始条目只此一处留痕」（见 `transcript::supplements` 的模块头）。
        .with_payload(serde_json::json!({
            "text": text,
            "count": count,
            "id": drained.id.clone(),
            "ids": ids.clone(),
        })),
    ) {
        // 落格失败**不静默**：消息仍会进这一次请求，但事实网格里没有它 ⇒ 下一轮起
        // 不可见。所以出声。
        //
        // `{:?}` 不是 `{e}`：`AppendError` 只 derive 了 `Debug`。
        crate::plugin_warn!(
            "session",
            "[Turn] 补充落格失败（仍注入本轮，下一轮起不可见）: {e:?}"
        );
    }
    let msg = ChatNode {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        role: Some(NodeRole::User),
        content: Some(MessageContent::Text(text.clone())),
        meta: Some(serde_json::json!({
            super::transcript::supplements::META_SUPPLEMENT: true,
            super::transcript::supplements::META_SUPPLEMENT_COUNT: count,
            super::transcript::supplements::META_SUPPLEMENT_IDS: ids,
        })),
        ..Default::default()
    };
    // 锁拿不到就**跳过回灌**，不 panic：这条补充**已经落格、已经进了本轮请求**，
    // 用户看得见——丢的只是落库镜像里的一条（重启后少它）。为这个 panic 不值。
    if let Ok(mut sink) = sink_msgs.lock() {
        sink.push(msg);
    }
    vec![crate::symbio_core::PromptMessage {
        role: "user".into(),
        text,
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}

/// **压缩入格 + 归档**（S2.7-b2）：把「本轮之前发生过一次压缩」变成一格事实
/// （`message × progressed` = `session.consolidated`），并把被它取代的前缀移出 live 段。
///
/// ## 为什么写方是这里，而不是压缩现场
///
/// 事实只能由**持写者令牌**的一方落格（[`EventWalStore`] 的单写者闸门）。压缩跑在
/// 步骤 3（`prepare_turn_inputs`）与步骤 6（`close_turn`），两处都**不持**令牌；令牌在
/// 执行轮里随 WAL 打开而取。压缩现场落格会拿到 `NotTheWriter`，而那是**静默**的——
/// 压缩照常生效、界面照常变短，只有事实源里少一条事件（于是切源之后，压缩在重建面
/// 上等于没发生过）。所以压缩把「压成了什么」寄存在
/// [`SessionContext::consolidated`](super::chat_loop::SessionContext::consolidated)
/// （见该字段文档），由本函数在**下一个**执行轮落格。
///
/// ## 幂等：按**内容**比对，不按事件 id
///
/// 同一轮里执行口可能被调多次，而压缩只发生一次 ⇒ 第二次不该再落一条。判据是与
/// live 段里**最近一份**快照逐 id 比对（[`crate::symbio_core::mutations_latest_snapshot`]）：
/// 一致 ⇒ 这份已经落过了。用内容而不是事件 id 判，是因为「该不该落」这件事由
/// **压成了什么**决定，而事件 id 只是它的名字。
///
/// ## 归档：快照即取代（b2 的语义）
///
/// 落格成功后调 [`EventWalStore::archive_before`]，把 `seq < 快照` 的事件整段移入归档段。
/// **「上下文压缩」与「WAL 无界增长」因此由同一个机制解决**（ADR-053 决策 6）：一次压缩
/// = 多一条快照事件 + 把被取代的前缀移走。归档段不删——ADR-053 迁移表 b2 行的
/// 「零回退损失」正靠它（各段按号序拼接 + live，逐条不重不漏 = 原 WAL）。
///
/// ## 载荷带**轮次水位**（`turns`）
///
/// 归档把被取代的轮次移出 live 段，而轮号是数 live 段里的 `user.message` 得来的
/// （[`count_user_messages`]）⇒ 不补水位，压缩之后轮号会**从 1 重新数起**，而
/// `u-{turn}` 是幂等键：重号会让新轮的用户格撞 `Duplicate` 静默落不进去。所以快照
/// 必须把「我取代了多少轮」一起带走——水位是历史状态的一部分，不是派生细节。
///
/// ## 失败一律**不致命**
///
/// 落格失败 ⇒ 告警并放弃归档（live 段保持完整）。归档失败 ⇒ 告警。两者都不回错给
/// 调用方：压缩**已经发生**（消息列表已重写、界面已变短），把它升级成整轮失败等于
/// 用「记不下来」换「答不出来」——与缺口 4 的汇报落格同一条口径。
///
/// 返回新落快照事件的 `seq`（`None` = 没有要落的，或这份已经落过）。
///
/// 入参是 `principal` 而不是 `ActorSpec`：本格只用到主体名，而两个调用点一个在轮内
/// （有 `ActorSpec`）、一个在轮外（只有名字）——收窄到真正用到的那一项。
pub(crate) fn consolidate_into_wal(
    store: &EventWalStore,
    messages: &[ChatNode],
    principal: &str,
) -> Option<Seq> {
    let existing = store.range(Seq::new(0));
    if let Some((_, prev)) = crate::symbio_core::mutations_latest_snapshot(&existing) {
        if same_message_ids(&prev, messages) {
            return None;
        }
    }
    // 被取代前缀的末号 = 此刻 live 段的最后一条（新事件尚未落）。事件 id 取它：
    // 同一份历史只压一次 ⇒ 同一个 id ⇒ 重放撞幂等键（`Duplicate`）而不是多出一条
    // 快照；号空间单调 ⇒ 不同压缩点必不同 id。
    let head = store.head().value();
    let superseded_last = head.saturating_sub(1);
    // 水位 = 被取代前缀里数得出的轮数（**同一个** `count_user_messages`，故套娃压缩
    // 会逐层累加：上一份快照的 `turns` 已计入它的前缀，本份自然接得上）。
    let turns = count_user_messages(&existing);
    let event = Event::pending(
        format!("v2sc-{superseded_last}"),
        crate::symbio_core::EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        // `turn = 0`：压缩不属于任何一轮——它发生在轮**边界**上，被取代的是**多轮**的
        // 历史（与 `memory.*` 同一约定，见 plan/01 §6）。
        0,
        principal,
    )
    .with_ts(crate::symbio_core::clock_now_ms())
    .with_payload(serde_json::json!({
        "messages": messages,
        "turns": turns,
    }));
    // 溯源锚 = 被取代前缀的末号（「这份快照是在那段历史之上压出来的」）。空 WAL
    // （迁移前的历史只存在于 `messages.json`）时**无锚可指** ⇒ 不设：I2 不要求
    // `progressed` 带溯源，编一个 `0` 反而是假锚。
    let event = if head > 0 {
        event.with_produced_by(superseded_last)
    } else {
        event
    };
    let seq = match store.append(event) {
        Ok(seq) => seq,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Session] 压缩入格失败（压缩已生效，事实源缺这一格）: {e:?}"
            );
            return None;
        }
    };
    // 快照即取代：被取代的前缀移出 live 段（归档段不删）。失败**不回错**——归档是
    // 增长优化、不是正确性前提；live 段一字未动（见 `archive_before` 的崩溃安全说明）。
    match store.archive_before(seq) {
        Ok(_) => {}
        Err(e) => crate::plugin_warn!(
            "session",
            "[Session] 压缩归档失败（live 段保持完整，仅未缩短）: {e}"
        ),
    }
    Some(seq)
}

/// 两份消息列表的 id 序列是否一致（压缩入格的幂等判据）。
///
/// **只比 id、不比正文**：快照的语义是「这份列表**取代**了它之前的一切」——判据是
/// **身份**，不是内容。正文在快照落格之后仍可被 `message.edited` 改（那是另一条事实），
/// 把它也算进判据会让「同一份快照」在编辑之后不再相等。
fn same_message_ids(a: &[ChatNode], b: &[ChatNode]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.id == y.id)
}

/// **轮外**压缩落格（S2.7-b2）：给 `context::retry_compaction` 用。
///
/// ## 为什么它需要一条自己的路
///
/// 用户重试压缩跑在 turn 循环**之前**（`process_resume` 在循环前调用它，之后一律
/// `Done`），所以没有「下一个 [`execute_turn`]」可以寄存——那条路唯一能落格的时刻
/// 就是它自己。此刻无人持写者令牌（循环还没开始），于是自己开一次 WAL 并交给
/// [`consolidate_into_wal`]——**落格实现仍是同一个**，差别只在谁持令牌。
///
/// ## 失败只告警
///
/// 打不开 / 拿不到令牌（同会话上有并发请求）⇒ 只告警并放弃。口径与轮内一致：压缩
/// **已经生效**（消息列表已重写、界面已变短），把它升级成整轮失败等于用「记不下来」
/// 换「答不出来」。
pub(crate) fn consolidate_outside_turn(
    session: &PersistentChatSession,
    messages: &[ChatNode],
    principal: &str,
) {
    let Some(dir) = session.session_dir() else {
        return;
    };
    let wal = dir.join(super::paths::V2_WAL_FILE);
    let store = match EventWalStore::open(&wal) {
        Ok(s) => s,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Session] 轮外压缩入格：WAL 打开失败（压缩已生效，事实源缺这一格）: {e}"
            );
            return;
        }
    };
    consolidate_into_wal(&store, messages, principal);
}

/// **定稿答话轮**（缺口 5）：`classify` 判 `Answered` ⇒ 本轮**一个模型请求都不发**，
/// 但轮次事实（用户格 + 收束格）仍须由 v2 运行器落。
///
/// ## 它解决的是什么
///
/// `Answered` 判决在 `chat_loop` 步骤 2 就短路收尾，**根本不进 [`execute_turn`]**；
/// 而 `EVENT_USER_MESSAGE` 是**运行器落的**。⇒ 那一轮在事实网格里**一格都没有**：
/// 没有用户格，也没有 `chat.assistant.final`。于是下一轮 prompt 里这次问答**整个消失**，
/// `session/stats` 的轮次计数也不含它——而 `messages.json` 里有（`context.messages` 落库）
/// ⇒ **两条真源**。完全静默：不报错、不告警、既有断言全绿。判据 = e2e `t22`。
///
/// ## 为什么走运行器，而不是本函数自己写那两格
///
/// 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由运行器守着。插件自己
/// 补写就成了**第二个轮次事实写方**——同轮两份记账的假象随之回来。所以答话交给运行器
/// （`TurnInput::final_reply`），格仍由它落。
///
/// ## 与 [`execute_turn`] 的两处刻意差异
///
/// 1. **不写派生事实**：`record_learning` / `record_derived` 一律不调——`Answered` 轮没有
///    工具交换、没有模型参与，`finish_turn` 直接收尾，没有派生副作用可记。
/// 2. **不上线 UI 帧**：答话节点已由 `apply_verdict` 推进 `context.messages`
///    （`dialog_node`，`exclude_from_context = false`），由 `finish_turn` 落库 ⇒ 这里用
///    [`SilentDeltas`]；再发一次就是同一条答话重复上屏。
///
/// ## `llm` / `tok` 是 `run_with_tools` 的端口，不是本路的依赖
///
/// 短路发生在**用到它们之前**（`TurnRunner::run_with_tools` 步骤 1b）——这一轮一次模型
/// 往返都不发。仍然照签名传，是因为「工具轮入口」只有那一个：另开一个入口就得把
/// 「用户格 + 收束格」的落格逻辑写第二遍，那才是真正的重复。
///
/// 形参到 8 个（`consolidation` 是 S2.7-b2 加的）：与 [`execute_turn`] 同一条口径
/// ——「一轮的全部输入」本就是一个宽签名，收成结构体只会把同一次调用拆成两处。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_final_reply_turn(
    session: &PersistentChatSession,
    ctx: &Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    provider: Arc<dyn ModelProvider>,
    abort: ExecAbortSignal,
    user_text: &str,
    user_node_id: Option<String>,
    user_meta: Option<serde_json::Value>,
    reply: &str,
    // 答话**对话面节点**的身份与标记（`id` / `reason` / `exclude_from_context`）：
    // 收束格据它复刻那条根级节点（`Answered` 答话 `exclude = false`）；必须随
    // 事实入格（WAL 为唯一事实源，见 `TurnInput::dialog`）。
    dialog: super::turn_runner::DialogFacts,
    // 生成这次答话的**思考事实**（`(reasoning_id, reasoning)`）：措辞产线的模型
    // 往回是这条轮次里唯一一次真实推理，不入格则 `Reason` 节点对普通答话轮从未
    // 存在（AMD MiMo 实测 2026-10-13）。模板产线 / 生成失败退模板 ⇒ `None`。
    reasoning: Option<(String, String)>,
    // 本轮之前发生过压缩 ⇒ 压缩后的整份消息列表（S2.7-b2，与 [`execute_turn`] 同源）。
    // 答话轮在步骤 2 就短路收尾，**不进** [`execute_turn`]，所以它也得自己落这一格
    // ——否则「上一轮末尾压过一次、这一轮是答话轮」时那次压缩永远不入格。
    consolidation: Option<&[ChatNode]>,
) -> Result<(), PluginError> {
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    let wal = dir.join(super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal)
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?;
    // **补落寄存的变更**（T-055）：答话轮同样持写者令牌，故它也是一个落格窗口。
    // 与 `execute_turn` 同序（在 `snapshot` 之前——本函数后面的 turn 号口径从切片折）。
    // 复用的是**上面这个持令牌的 `store`**，不是另开一个（令牌是文件锁，见 `drain_parked`）。
    super::v2_messages::drain_parked(&dir, &store);
    let snapshot = store.range(Seq::new(0));
    // turn 号口径与 [`execute_turn`] 同源（WAL 内既有 `user.message` 计数）——两处各写
    // 一套必然在「答话轮之后紧接着一个新轮」时错开一号。
    let turn_no = count_user_messages(&snapshot);
    // 身份与档位取值点同 [`execute_turn`]：主体 = 会话选定的那个 agent；档位 = 深度
    // （`Answered` 轮没有可跳过的模型调用，不属于反射档）。规格从授权表行取
    // （`request_actor_spec`，能力集 / 模式不再是平凡值）。
    // 作用域与身份同批取（[T-105]）：子会话挂父主体，顶层会话恒 `Root`。
    let scope = super::chat_loop::request_scope(session).await;
    let actor = super::chat_loop::request_actor_spec(ctx.as_ref(), LatencyTier::Deep, scope);
    let principal = actor.principal.clone();
    // 压缩入格 + 归档：与 [`execute_turn`] 同序（在 `turn_no` 之后、运行器之前）。
    if let Some(msgs) = consolidation {
        consolidate_into_wal(&store, msgs, &principal);
    }
    let llm = ProviderLlmAdapter::for_turn(provider, "", abort.clone());
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: turn_no,
        text: user_text.to_string(),
        tier: LatencyTier::Deep,
        // 不生成 ⇒ 没有 prompt，也就没有窗口可裁。
        window_turns: None,
        // `Answered` 只在轮首成立（`tool_rounds == 0`），不带恢复锚点。
        resume: None,
        actor,
        prefix: None,
        inject: None,
        final_reply: Some(reply.to_string()),
        // 答话**对话面节点**的身份与标记：收束格据它复刻那条根级节点。答话轮没有
        // 流式过程 ⇒ 无半截可记：载荷 `turn_id` 记空串（重建侧按 `dialog.id` 与
        // 事件 id 复刻），取数口按有无思考事实决定挂不挂（见下方 `nodes`）。
        dialog: Some(dialog),
        // 答话轮**不收束前的开场白**：`Answered` 在轮首就判定了（`Spoke` 是
        // `Escalate` 的效应），本入口只处理已定稿的答话。
        opening_text: None,
        opening_dialog: None,
        turn_root_id: String::new(),
        user_node_id,
        user_meta,
        // 取数口：**恒定快照**——答话轮没有流式过程（措辞插件的模型往返是静默的，
        // 此刻已结束），思考事实是调用方搬运来的定值。挂上它，1b 收束格的
        // `produced` 载荷才带 `reasoning` / `reasoning_id`（重建据此复刻 Reason 节点）。
        nodes: reasoning.map(|(reasoning_id, reasoning)| {
            let facts = super::turn_runner::StreamNodeFacts {
                text_id: None,
                text: None,
                reasoning_id: Some(reasoning_id),
                reasoning: Some(reasoning),
            };
            std::sync::Arc::new(move || facts.clone()) as super::turn_runner::NodeFactsFn
        }),
        // 定稿答话轮不经过工具循环的闸门空隙——它一次请求都不发（`final_reply` 走
        // 1b 分支直接收束）。但信号照传：若用户在答话落格前按下停止，1c 的中止检查
        // 会在落完用户格后接管，与其它轮同一条路径（终止是状态，不是特例）。
        abort: Some(abort),
    };
    TurnRunner
        .run_with_tools(&store, &llm, &tok, input, Arc::new(SilentDeltas), &[], None)
        .await
        .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;
    Ok(())
}

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
        user_node_id,
        user_meta,
        window_turns,
        tools,
        resume,
        recalled,
        supplements,
        on_round,
        supplemental_no,
        skill_obs,
        skill_hits,
        prefix,
        consolidation,
        opening_text,
        opening_dialog,
        presentation_tree,
    } = req;
    // 事实源：per-session v2 WAL（与运行器落格同一个文件——一份网格）。
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    let wal = dir.join(super::paths::V2_WAL_FILE);
    // `Arc` 是给轮内注入闭包用的（`TurnInjector` 是 `'static` 闭包，而
    // `WalStore` 不 `Clone`）。**只有它 clone**——`Arc` 共享同一份，注入的落格
    // 与本函数末尾的落格进的是同一个文件，append-only 下不会打架。
    let store = std::sync::Arc::new(
        EventWalStore::open(&wal)
            .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?,
    );
    // **补落寄存的变更**（T-055）：一轮正在跑时它持着写者令牌，此刻来的编辑 /
    // 截断拿不到令牌 ⇒ `NotTheWriter` ⇒ 该变更被寄存在进程内（`v2_messages::PARKED`）。
    // 本函数是**持令牌的那一刻**，寄存的变更在此补落。
    //
    // 位置在 `snapshot` 之前：本函数后面的所有判据（turn 号 / 续写锚 / 幂等键）
    // 都从 `store.range(...)` 折，补落改变了 live 段 ⇒ 必须在读切片之前落完，
    // 否则本轮会站在一份**缺了那次编辑**的状态上做判断。
    super::v2_messages::drain_parked(&dir, &store);
    let snapshot = store.range(Seq::new(0));

    // turn 号与续写锚点：
    // - 新开轮：号 = WAL 内既有 `user.message` 计数；
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
                        overflow_notice: None,
                    },
                    text: r.text.clone(),
                    // 节点身份（调用 / 轮根 / 结果）随恢复方直给的取值入格——续写轮的
                    // `artifact.added` 复用它们，重建才把结果挂回**原来那张卡**之下。
                    call_id: Some(r.call_id.clone()),
                    turn_id: Some(r.turn_id.clone()),
                    result_id: Some(r.result_id.clone()),
                }),
            )
        }
    };

    // 折进的补充消息的收集器（缺口 3）：注入闭包（`'static`）往它 push，
    // 执行完随 `V2TurnResult` 回灌进 `context.messages`（落库动作就是
    // `context.messages.push(merged)`）。不回灌就只活在这一次请求里，
    // 重启即丢，而 t19 等的正是落库那条。
    let collected: Arc<std::sync::Mutex<Vec<ChatNode>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // 流式桥：分帧侧（同步回调）与发射侧（异步任务）经 unbounded 通道解耦。
    // 通道在桥 drop（生成结束）后关闭，接收端自然退出。
    let tx = Arc::new(FrameQueue::new(UI_FRAME_CAPACITY));
    let bridge = Arc::new(UiBridge {
        tx: tx.clone(),
        root_id: root_id.to_string(),
        node: Arc::new(Mutex::new(None)),
        reasoning: Arc::new(Mutex::new(None)),
    });
    let emit_sink = sink.clone();
    let root_id_owned = root_id.to_string();
    let emitter = tokio::spawn(async move {
        while let Some(frame) = tx.pop().await {
            let message = match frame {
                UiFrame::Snapshot {
                    id,
                    parent,
                    text,
                    kind,
                } => {
                    let mut node = kind.shape(ChatNode {
                        id,
                        parent_id: Some(parent),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Streaming),
                        ..Default::default()
                    });
                    super::message_build::detach_if_flat(
                        &mut node,
                        &root_id_owned,
                        presentation_tree,
                    );
                    node
                }
                UiFrame::Delta { id, text } => ChatNode {
                    id,
                    delta: Some(text),
                    ..Default::default()
                },
                UiFrame::Finalize {
                    id,
                    parent,
                    text,
                    kind,
                } => {
                    let mut node = kind.shape(ChatNode {
                        id,
                        parent_id: Some(parent),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Completed),
                        ..Default::default()
                    });
                    super::message_build::detach_if_flat(
                        &mut node,
                        &root_id_owned,
                        presentation_tree,
                    );
                    node
                }
                UiFrame::Barrier(done) => {
                    let _ = done.send(());
                    continue;
                }
                // 工具调用快照：model 插件转写面的事实原样到达，这里不做任何改写
                // （改写 = 第二份身份形状，分发方与落库用的是同一份）。
                UiFrame::Tool(m) => *m,
            };
            emit_sink.emit(message).await;
        }
    });

    // 本轮是否走**反射档**（S11 快路）：命中一条已编译技能 ⇒ 以技能正文收束、
    // 一次模型调用都不发生。两关缺一不可：
    //
    // - **逐字命中**（`take_match`：判据是数据，更宽的匹配接进来时改的是它）；
    // - **只对新开轮**：续写轮**不新开用户格**（运行器复用 `resume.user_seq`），而
    //   反射档的落格形状就是新开一轮（`u-{turn}`）⇒ 续写轮走它会撞幂等键
    //   （`AppendError::Duplicate` → Failed 出口），既完不成收束、也把本轮变成错误。
    //   语义上同样说不通：续写要跑的是**刚被用户批准的那次工具**，不是拿技能顶掉它。
    //   故 `resume_anchor` 非空时一律不走快路（判定根本不算，不是算了再丢）。
    // 先做**一次**匹配，再问表"这一格要不要走快路"。判据在
    // `loop_registry` 的 `policy.reflex`（`resume_allowed ∧ has_match`），
    // 本处只提供两个布尔并执行命中结果——`hit` 全程留在本地，表不接触它。
    let matched = super::v2_skills::take_match(skill_hits, user_text);
    let reflex_state =
        super::loop_registry::LoopState::reflex(resume_anchor.is_none(), matched.is_some());
    let hit = match super::loop_registry::builtin()
        .decide(&reflex_state)
        .map(|h| h.capsule)
    {
        Some(super::loop_registry::Capsule::REFLEX) => matched,
        _ => None,
    };

    // 档位：命中 ⇒ 反射（`budget_ms = 80`、开轮格的 `tier = reflex`），未命中 ⇒ 深度。
    // `tier` 与 `actor.budget_ms` 是**同一个事实**（一个入格、一个随主体走），
    // 故由前者派生后者——两处各写一遍迟早对不上（S11 §3 的 ActorSpec 行）。
    //
    // 读方（T-082 批①）：派生出的数值随主体走，`turn_runner` 开轮时以**数值**写入
    // 载荷 `budget_ms`，`invariants::declared_budget_ms` 优先读它 ⇒ 该字段不再是
    // "算了却看不见"。`tier` 仍入格（兜底率按层统计的口径锚），且作为缺 `budget_ms`
    // 时的回退路径保留——两条路径读的是同一条事实的两种形态。
    let tier = if hit.is_some() {
        LatencyTier::Reflex
    } else {
        LatencyTier::Deep
    };
    // 身份进事件（[plan/11 批 1](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") ①）：
    // 本轮以**会话选定的那个 agent** 为主体，规格从**授权表行**取（`request_actor_spec`）：
    // 能力集与模式不再是 `trivial` 的空表平凡值——它们与写侧闸判的是同一张表
    // （判的对象 = 写的对象 = 表授予的对象）。未选智能体 ⇒ `agent:main`
    // （S08 §4 平凡值，与接线前逐字一致）。
    //
    // 另存一份 `principal`：`actor` 随 `input` 被移动进运行器，轮末的记忆与学习
    // 写方（`record_learning`）仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    // 作用域（[T-105]）：`agent_run` 派生的子会话挂父主体，顶层会话恒 `Root`。
    // 与身份同批取——两者都经 `ctx` / 元数据解析，分开取会让「主体是 A、作用域按 B 算」。
    let scope = super::chat_loop::request_scope(session).await;
    let actor = super::chat_loop::request_actor_spec(ctx.as_ref(), tier, scope);
    let principal = actor.principal.clone();
    // 来源主体（ADR-067）：`commitment.*` 的 `to`。在此刻取而不是轮末——`ctx` 会被
    // 移进工具分发口（`SessionDispatchPort`），轮末已经拿不到它。
    let to = super::chat_loop::request_source_principal(ctx.as_ref());

    // ── 压缩入格 + 归档（S2.7-b2）─────────────────────────────────────────
    //
    // **必须在 `(turn_no, resume_anchor)` 之后**：归档把被取代的轮次移出 live 段，而
    // `last_open_turn`（续写轮靠它找未收束的那一轮）与 `count_user_messages`（轮号）
    // 都是在归档**之前**的 live 段上算出来的。顺序反了，续写会报「找不到未收束的轮次」。
    //
    // 也必须**在运行器之前**：本轮自己的用户格是运行器落的，先归档才能让
    // 「本轮之前的一切」被完整取代（否则快照与它后面的新事件之间会夹着被取代的旧事件，
    // live 段不再自洽）。
    if let Some(msgs) = consolidation {
        consolidate_into_wal(&store, msgs, &principal);
    }

    let mut produced = Vec::new();
    // 工具执行层交回的收束派生事实（承诺 / 任务表 / 熔断）——只有工具轮（深度档）才有，
    // 反射档与无工具轮保持默认空值。轮末随收束落格（写方 = `v2_facts::record_derived`，
    // 见下方）。
    let mut derived = super::v2_tools::DerivedFacts::default();
    let outcome = match &hit {
        // ── 反射档（S11 §2）：**没有 `llm` 形参**的那条路 ────────────────────
        // "反射档调模型"在类型上写不出来（`run_reflex` 只收 `RuleOnly`）——
        // 这正是 `docs/004-plan/verify/latency_gate.rs::assemble(Reflex)` 的落地形态
        // （反射档装不进 LLM 字段），而不是"装配了模型但约定不调它"。
        Some(h) => {
            crate::plugin_info!(
                "session",
                "[v2-exec] 技能快路命中（skill_id={}），本轮不调模型",
                h.skill_id
            );
            let tok = TokenIssuer::issue_reflex();
            TurnRunner
                .run_reflex(
                    &*store,
                    &tok,
                    turn_no,
                    &actor,
                    user_text,
                    &h.content,
                    bridge.clone(),
                )
                .await
        }
        // ── 深度档：完整推理（与接线前逐字同路）─────────────────────────────
        None => {
            let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, abort.clone());
            let tok = TokenIssuer::issue_deep();
            let input = TurnInput {
                turn: turn_no,
                text: user_text.to_string(),
                tier,
                window_turns: Some(window_turns),
                resume: resume_anchor,
                actor: actor.clone(),
                prefix,
                // 轮内注入（缺口 3）：用户中途说的话折进**本轮**。
                //
                // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
                // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
                // `InboxItem` 都在 `plugins/session`），而落格也不需要 core 参与：
                // 本函数自己就开着 `store`。
inject: super::round_hook::round_hook(supplements, on_round).map(|(drain, report)| {
                    let store = store.clone();
                    let actor = actor.clone();
                    // 折进的消息往收集器 push（末尾随 `V2TurnResult` 回灌进落库镜像）。
                    let sink_msgs = collected.clone();
                    // 汇报「产生即上线」借本轮那条流式桥送帧（`UiBridge::on_tool_frame`）。
                    // 克隆而非搬走：`bridge` 之后还要交给运行器，而函数末尾的 `drop(bridge)`
                    // 是发射端退出**唯一**的信号——多养一份它，等于把那个信号推迟到闭包死。
                    // 闭包随 `TurnInput` / `drop(dispatcher)` 在本轮内死去，因此那个信号
                    // 不会被推过收束点；这条时序是 hang 级别的（每个 v2 工具轮都要过它），
                    // 由 `round_report_reaches_the_live_surface_before_turn_end` 钉住。
                    let live = bridge.clone();
                    std::sync::Arc::new(
                        move |anchor: Option<u64>|
                            -> std::pin::Pin<
                                Box<
                                    dyn std::future::Future<
                                        Output = Vec<crate::symbio_core::PromptMessage>,
                                    > + Send,
                                >,
                            > {
                            // `async move` 块按值捕获，所以这里先各 clone 一份——
                            // 否则第一次调用就把捕获物搬走，闭包退化成 `FnOnce`。
                            let drain = drain.clone();
                            let report = report.clone();
                            let store = store.clone();
                            let actor = actor.clone();
                            let sink_msgs = sink_msgs.clone();
                            let supplemental_no = supplemental_no.clone();
                            let live = live.clone();
                            Box::pin(async move {
                            // 同步取件：队列与配置都在 tokio RwLock 里，用 `blocking_read`
                            // 桥过来。
                            //
                            // 抽干口是**闭包**：它内部用 `blocking_read` 跨过 tokio 锁，
                            // 所以多线程 runtime 上要 `block_in_place`（**单线程 runtime 上
                            // 调用会 panic**）——所以先判 flavor。
                            let batch = match tokio::runtime::Handle::try_current() {
                                Ok(h)
                                    if h.runtime_flavor()
                                        == tokio::runtime::RuntimeFlavor::MultiThread =>
                                {
                                    tokio::task::block_in_place(|| drain())
                                }
                                // 单线程 / 无 runtime（单测）：裸调（闭包内部是阻塞读）。
                                _ => drain(),
                            };
                            // 顺序：**先抽干、后汇报**（同一条纪律）——抽干会把静默时钟
                            // 归零，反过来就会出现「用户刚说完话，助手抢着报了一句进度」。
                            let injected = supplement_into_request(
                                batch,
                                anchor,
                                &store,
                                &actor,
                                turn_no,
                                &sink_msgs,
                                &supplemental_no,
                            );

                            // 轮边界汇报（缺口 4）：判定与措辞在 chat_loop 那边（它有编排器），
                            // **落格在这里**（本函数开着 store）。所以回调交回来的是一份
                            // **草稿**，不是已落的事实——落格这一半不外包出去。
                            //
                            // **落格即上线**（T-136 的另一半）：这句话的设计目的就是「本轮还要
                            // 跑很久，先报一句」，只落格不下发就等于把它压到轮末、把目的抵消。
                            // 节点由 `to_message()` **现造一次**，id 同写进落格载荷——重建侧
                            // （`rebuild::reported_node` 走 `event_payload_node_id`）读的就是它，
                            // 于是实时面与重建面是同一个身份，轮末落库镜像再送一次时按 id 合并，
                            // 不多一张卡（与 `op-*` 那条开场白同纪律）。
                            if let Some(report) = report {
                                if let Some(draft) = report(anchor).await {
                                    match anchor {
                                        Some(anchor) => {
                                            let no = supplemental_no.fetch_add(
                                                1,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            let node = draft.to_message();
                                            if let Err(e) = store.append(
                                                crate::symbio_core::Event::pending(
                                                    format!("rp-{turn_no}-{no}"),
                                                    crate::symbio_core::EVENT_ASSISTANT_REPORTED,
                                                    crate::symbio_core::Entity::Turn,
                                                    crate::symbio_core::Verb::Asserted,
                                                    turn_no,
                                                    &actor.principal,
                                                )
                                                .with_produced_by(anchor)
                                                .with_payload(serde_json::json!({
                                                    "text": draft.text,
                                                    "tool_rounds": draft.tool_rounds,
                                                    "quiet_ms": draft.quiet_ms,
                                                    "id": node.id.clone(),
                                                })),
                                            ) {
                                                crate::plugin_warn!(
                                                    "session",
                                                    "[Turn] 汇报落格失败（仍回灌落库镜像，下一轮起模型看不见）: {e:?}"
                                                );
                                            }
                                            if let Ok(mut sink) = sink_msgs.lock() {
                                                sink.push(node.clone());
                                            }
                                            live.on_tool_frame(&node);
                                        }
                                        None => crate::plugin_warn!(
                                            "session",
                                            "[Turn] 汇报无溯源锚点，本轮丢弃（I2：断言类事件必须带溯源）"
                                        ),
                                    }
                                }
                            }

                            injected
                        })
                    }) as crate::plugins::session::turn_runner::RoundInjector
                }),
                // 本路**恒**走生成：定稿答话轮（缺口 5）不经这里——它一个模型请求都不发，
                // 由 `execute_final_reply_turn` 单独入口落格（见该函数）。
                final_reply: None,
                // 本路恒走生成 ⇒ 没有对话面节点（定稿答话轮不走这里）。
                dialog: None,
                // 对话面开场白（T-056，`Escalate` 首响）：由 `chat_loop::apply_verdict`
                // 产出、随 `V2Turn` 递进来，运行器据它落 `chat.assistant.opening` 格
                // （开场白后面照常进工具循环，故不占收束格）。
                opening_text: opening_text.map(str::to_string),
                opening_dialog,
                // 落格身份三件套：本轮 Turn 根 id（载荷 `turn_id`）、用户消息节点 id
                // （用户格载荷记它）、流式节点取数口。取数口是**快照闭包**：运行器每轮
                // 收束 / 进展 / 兜底时问一次「此刻桥上攒着哪两个节点」，中途正文与思考
                // 因此逐轮入格（半截中止时它们是唯一幸存者）。
                turn_root_id: root_id.to_string(),
                user_node_id,
                user_meta,
                nodes: Some(std::sync::Arc::new({
                    let bridge = bridge.clone();
                    move || {
                        let (text_id, text) = bridge
                            .current_node()
                            .map(|(id, acc)| (Some(id), Some(acc)))
                            .unwrap_or((None, None));
                        let (reasoning_id, reasoning) = bridge
                            .current_reasoning()
                            .map(|(id, acc)| (Some(id), Some(acc)))
                            .unwrap_or((None, None));
                        crate::plugins::session::turn_runner::StreamNodeFacts {
                            text_id,
                            text,
                            reasoning_id,
                            reasoning,
                        }
                    }
                })
                    as crate::plugins::session::turn_runner::NodeFactsFn),
                // 中途中止信号（ADR-052 决策 1）：闸门与首请求之间的空隙由运行器
                // 接管——落完用户格、发第一个请求前检查，已中止则走中止收束格。
                abort: Some(abort.clone()),
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
                presentation_tree,
            );
            let dispatch_ref: &dyn DispatchPort = &dispatcher;
            let r = TurnRunner
                .run_with_tools(
                    &*store,
                    &llm,
                    &tok,
                    input,
                    bridge.clone(),
                    tools,
                    Some(dispatch_ref),
                )
                .await;
            // 分发方产生的消息（工具轮）——先取走再释放，避免它随分发方一起消失。
            produced = dispatcher.take_produced();
            // 收束派生事实出参同样先取走（承诺 / 任务表 / 熔断）——轮末随收束落格。
            derived = dispatcher.take_derived();
            drop(dispatcher);
            r
        }
    }
    .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;

    // 本轮**未定格**的正文与推理节点必须在桥 drop 之前取走：它们的收束帧由
    // chat_loop 的 `finalize_assistant_turn` 发出（用 `TurnOutput` 里的这两个 id），
    // 而**落库**则由 `TurnOutput::into_messages` 按同一批 id 建节点。
    //
    // 工具轮的那几轮在 `v2_tools::dispatch` 里已经定格并取走了 ⇒ 这里拿到的是
    // **最后一轮**（无工具调用那一轮）的节点；没有推理就是 `None`（`on_reasoning`
    // 一次都没被调过），不是空节点。
    let (child_id, child_text) = bridge.current_node().unwrap_or_default();
    let (reasoning_child_id, reasoning_text) = bridge.current_reasoning().unwrap_or_default();
    // 桥归零 ⇒ 通道关闭 ⇒ 发射端排空后退出。
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：收束格已由运行器落格（`f-{turn}`，载荷 `status: "aborted"` + `terminated`），
    // 出口走 Aborted——由 chat_loop/消费循环落库为 `NodeStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`）。
    // 记忆与学习也**不写**——中止的轮次没有定稿产出，没有可固化的东西。
    //
    // **半截产出随出口带走**（`partial`）：末轮正文 / 思考此刻还只攒在桥上，丢了它们
    // 用户中断前看到的那半句话就消失。消费循环把 `partial` 收尾入格
    // （按 id 去重、整条覆盖、孤儿丢弃，见 `persist_failure`）；工具轮已定格的节点
    // （`produced`）同走这条通道——中止出口不再回灌 `V2TurnResult`，不带上就没了。
    if outcome.aborted {
        let mut partial = produced;
        if !child_text.is_empty() {
            let mut n =
                super::message_build::llm_build_text_node(root_id, &child_text, None, child_id);
            super::message_build::detach_if_flat(&mut n, root_id, presentation_tree);
            partial.push(n);
        }
        if !reasoning_text.trim().is_empty() {
            let mut n = ChatNode {
                id: reasoning_child_id,
                parent_id: Some(root_id.to_string()),
                role: Some(NodeRole::Assistant),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(reasoning_text.clone())),
                status: Some(NodeStatus::Completed),
                timestamp: Some(crate::symbio_core::clock_now_ms()),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING);
            super::message_build::detach_if_flat(&mut n, root_id, presentation_tree);
            partial.push(n);
        }
        return Err(PluginError::Aborted { partial });
    }

    // ── 收束派生事实（两半）：承诺 / 任务表 / 熔断 + 记忆与学习 ────────────────
    //
    // **本步的存在理由**：轮次事实由运行器原生入格，这两半都**不是轮次事实**，而是
    // 本轮的**派生副作用**——运行器一处都不写，写方就是这里的 `record_derived` /
    // `record_learning`（溯源锚 = 运行器原生写的 `u-{turn}` 格）。不在这里补，
    // 代际立约（S08）/ 任务表（S7）/ 熔断（S9 §6 验收 2）/ 长期记忆（S06）/
    // 技能自我改进（S11）就整体失效。
    //
    // 兜底收束的 `response` 为 `None`：兜底说明这条路没走通，固化它等于把失败写成
    // 套路。写侧闸拒（`closure_denied`，见运行器的 `closure_granted`）：收束格
    // **未入格** ⇒ 本轮的派生事实**也不写**（承诺 / 任务 / 熔断 / 记忆全不写）。
    // 用户的答案照旧经 `produced` 回给调用方——收束格是**记录**，不是呈现。
    if outcome.closure_denied {
        crate::plugin_warn!(
            "session",
            "[v2-exec] 收束被授权拒绝，本轮派生事实不入格（turn={turn_no}）"
        );
        // 被拒的兜底仍是**失败**（I3）：呈现走 Failed 出口，与未拒时同形。
        if outcome.fell_back {
            return Err(PluginError::InternalError(outcome.text));
        }
        return Ok(V2TurnResult {
            output: TurnOutput {
                text: outcome.text,
                reasoning: reasoning_text,
                response_text_child_id: child_id,
                reasoning_child_id,
                // 实测用量直通：校准反馈的唯一数据源（缺口 6）。此前这一格
                // 恒为 `None`，校准比冻结在初值 1.0，压缩预检把本可成功的
                // 摘要请求误判成「注定超限」。
                usage: outcome.usage,
                // 末轮原因直通（T-144）：`..Default::default()` 会把 finish 静默填成
                // Stop，chat_loop 的长度戳与自动续写判据因此读不到真值。反射档 /
                // 兜底 / 中止没有模型轮 ⇒ Stop 就是诚实值。
                finish: outcome.finish,
                ..Default::default()
            },
            messages: produced,
        });
    }

    let response = (!outcome.fell_back).then_some(outcome.text.as_str());
    // 本轮开口格（`u-{turn}`）的 seq —— 记忆 / 任务 / 承诺的溯源锚。
    //
    // 读 `history()` 而非 `range()`（S2.7-b2）：这三个写方（`record_derived` 的
    // 承诺与任务表 / `record_learning` 的记忆三段）都是**累计口径**——它们的幂等
    // doc-link-allow D-004: 「曾经落过什么」问的是事实网格里有无该条记录，属数据语义
    // 判据与状态推导要看**曾经**落过什么。压缩归档把被取代的前缀移出 live 段后，
    // 只读 live 会让它们看不见压缩前的事实：幂等判据失效 ⇒ 重复落格，或状态
    // 丢失（例如压缩前开的任务在就绪集里消失）。`user_seq` 本身取的是**本轮**
    // 开口（在 live 段里），但同一次读取顺带把历史带上，省一次 IO。
    let all = store.history();
    match all
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE && e.turn == turn_no)
        .and_then(|e| e.seq.map(|s| s.value()))
    {
        Some(user_seq) => {
            // 承诺 / 任务表 / 熔断：写方是 `record_derived`。
            // 承诺失败只记日志不冒泡——轮次已收束，派生事实失败不该把成功的一轮说成
            // 失败（与记忆三段同一条口径）。
            //
            // `to` = 来源主体（ADR-067）：本主体向「这条消息的出处」立约。缺省落
            // 本机用户——本机路径与接线前一致（取值点见 `request_source_principal`，
            // 在 `ctx` 被移进分发口之前取好）。
            if let Err(why) = super::v2_facts::record_derived(
                &wal,
                &store,
                &all,
                turn_no,
                user_seq,
                &principal,
                &to,
                &format!("t{turn_no}"),
                &derived.delegations,
                &derived.tasks,
                &derived.breaks,
            ) {
                crate::plugin_warn!("session", "[v2-exec] 承诺入格失败（turn={turn_no}）：{why}");
            }
            // 淘汰回路（S13 §3，T-073）：**必须排在承诺落格之后**——它读的声誉投影
            // 要看得见本轮刚写下的那条违约（委托失败 ⇒ 声誉 −1 ⇒ 收回 assign.work）。
            // 顺序反了会让判据滞后一个轮次：那一轮里主体已经又派出去一轮。
            super::v2_facts::record_elimination(store.as_ref(), turn_no, user_seq);
            // 记忆与学习：同一函数（`record_learning`）。
            super::v2_facts::record_learning(
                &wal,
                // `store` 是 `Arc<WalStore>`（轮内注入闭包要 `'static`），而本函数
                // 收 `&WalStore` ⇒ 要一次显式 deref。写 `store.as_ref()` 而不是
                // `&*store`：后者会被 clippy 判 `needless_borrow`（它建议 auto-deref，
                // 但 auto-deref 到 `Arc` 本身，而 `Arc<WalStore>` 不实现 `Store`）。
                store.as_ref(),
                &all,
                turn_no,
                user_seq,
                user_text,
                &principal,
                &format!("t{turn_no}"),
                response,
                recalled,
                skill_obs,
                session.skill_compile_enabled(),
                crate::symbio_core::clock_now_ms(),
            );
        }
        // 锚缺失即**不写**并出声：静默锚在 0 上会把记忆挂到不存在的轮次，
        // 那比不写更坏（`produced_by` 是 I2 的判据）。
        None => crate::plugin_warn!(
            "session",
            "[v2-exec] 本轮用户格缺失，记忆与学习不入格（turn={turn_no}）"
        ),
    }

    // 兜底已入格（I3），失败呈现走 Failed 出口 + 重试。
    if outcome.fell_back {
        return Err(PluginError::InternalError(outcome.text));
    }
    // 等待用户（`outcome.awaits_user`）：本轮尚未了结（运行器只落了进展格
    // `status: "pending"`，收束格留给恢复幕），但**按正常收束返回**——待用户动作由
    // 分发方广播的 `user_prompt` 节点承载（`WaitingUserAction`），呈现是卡片 +
    // 恢复入口；恢复时经 [`TurnResume`] 续写同一轮（不重开用户格）。
    // 此处的 `text` 只是模型在工具轮说过的话，不是终答。

    // 折进的补充也要进落库镜像（缺口 3）：排在 `produced` **之前** ——
    // 用户先插话、工具结果后到，这是时间序（`persist_messages` 只写尾部增量，
    // 顺序错了落库就乱了）。
    // 同上：锁拿不到就用空的（落库镜像少这批补充，其余不受影响）。
    let mut all_messages: Vec<ChatNode> = collected
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default();
    all_messages.extend(produced);

    Ok(V2TurnResult {
        output: TurnOutput {
            text: outcome.text,
            reasoning: reasoning_text,
            response_text_child_id: child_id,
            reasoning_child_id,
            // 实测用量直通：校准反馈的唯一数据源（缺口 6），与上面被拒出口同一条。
            usage: outcome.usage,
            // 末轮原因直通（T-144）：与上面被拒出口同一条纪律。此前 `..Default::default()`
            // 把这一格静默填成 Stop，长度截断的自动续写在 v2 执行路径不可达。
            finish: outcome.finish,
            ..Default::default()
        },
        messages: all_messages,
    })
}

/// WAL 内 `user.message` 计数（新开轮的 turn 号；与桥同一口径）。
///
/// ## 为什么必须加上快照水位（S2.7-b2）
///
/// 压缩把被取代的轮次**移出 live 段**（[`consolidate_into_wal`] 的归档），而归档后的
/// live 段里可能一条 `user.message` 都不剩（只剩快照 + 本轮新落的事件）⇒ 只数 live 段
/// 会让轮号**倒退**：压缩之后从 1 重新数起。而 `u-{turn}` 是幂等键，重号会让新轮的
/// 用户格撞 `Duplicate` —— 那是**静默**的（那一轮在事实网格里没有用户格，下一轮 prompt
/// 里整段消失）。所以水位必须随快照一起带过来：快照载荷的 `turns` = 它取代了多少轮。
///
/// 口径仍是「数 `user.message`」，只是分成「快照之前（读它的 `turns`）」+
/// 「快照之后（数 live 事件）」两段相加——套娃压缩因此逐层累加，不会丢也不会重。
fn count_user_messages(events: &[Event]) -> u64 {
    let (start, base) = match crate::symbio_core::mutations_latest_snapshot(events) {
        Some((i, _)) => (i + 1, consolidated_turns(&events[i])),
        None => (0, 0),
    };
    base + events[start..]
        .iter()
        .filter(|e| {
            e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE
        })
        .count() as u64
}

/// 快照事件载荷里的**轮次水位**（[`EVENT_SESSION_CONSOLIDATED`] 的 `turns`）。
///
/// 缺失 / 非数 ⇒ `0`：那是旧版本写的快照（水位字段是 b2 才加的）。降级方向是
/// 「轮号从头数起」而不是「算出一个错的水位」——宁可保守。
fn consolidated_turns(e: &Event) -> u64 {
    e.payload.get("turns").and_then(|v| v.as_u64()).unwrap_or(0)
}

/// **已开未收束**的轮次（`turn × opened` 无同号收束）：返回 `(turn, 用户格 seq)`。
///
/// 与 core 的 `invariants::unresolved_turns`（C4）**刻意不同口径**：这里只认
/// final / fallback 收束格、**不认产物格**（C4 经 ADR-067 认）。原因：等待用户
/// 动作的那一轮（进展格不是收束格，但往往已落过产物格）的收束格刻意留给恢复幕
/// ——若这里也把「落过产物」当成已收束，恢复请求会报「找不到未收束的轮次」，
/// 审批 / 问答恢复整条链路断掉。C4 只负责「不再报它静默中断」，恢复锚仍按
/// final / fallback 找——两处口径的差异是 ADR-067 落地时的显式决策，不是漏改。
/// 取**最后**一个：恢复请求总是续写最近一次停顿。
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

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/plugins/session/v2_exec.rs::current_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::current_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::finalize_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::finalize_reasoning_node: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::on_delta: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::on_reasoning: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::send: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::close: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_exec.rs::pop: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
