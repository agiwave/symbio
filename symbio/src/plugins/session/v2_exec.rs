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
//! ## 反射档（S11 快路）：本档的第三条路，也是唯一不调模型的一条
//!
//! 开 `skill_fast_path` 且本轮发言**逐字命中**一条已编译技能时，本轮改走
//! [`TurnRunner::run_reflex`]——以技能正文收束，**一次模型调用都不发生**。
//! 它不是"又一条执行链"：产物落的是**同一对格子**（`u-{turn}` / `f-{turn}`），
//! 收束后仍走同一段 `record_learning`，收口（步骤 5–7）也完全共享；变的只是
//! **生成那一格**由谁产出（反射档没有 `llm` 形参 ⇒ "反射档调模型"写不出来）。
//!
//! 三条边界，缺一条都会静默走错：
//! - **只对新开轮**：续写轮不新开用户格（复用 `resume.user_seq`），而反射档的落格形状
//!   就是新开一轮 ⇒ 走它会撞幂等键（`Duplicate`），既完不成收束也把本轮变成错误；
//! - **只在 `full` 档**：跳过的是本档里的那一次模型调用，`bridge` / `off` 档没有可跳过的；
//! - **开关默认关**：关着时候选集恒空，本档与接线前逐字同路。
//!
//! 一处**诚实缺口**：反射档把 `actor.budget_ms` 按档位派生
//! （`LatencyTier::Reflex.budget_ms()`），但该字段在生产里仍**没有读方**
//! （`invariants::declared_budget_ms` 读的是开轮载荷的 `tier` 字符串）。派生出的值
//! 今天是"算了却看不见"——读方归 `invariants` 那一侧（它读 `tier` 还是读这个字段，
//! 是那边的决定），见 `execute_turn` 里派生点旁的注记。
//!
//! ## 收束派生事实（本档已覆盖）
//!
//! 轮次事实由运行器原生入格，`chat_loop` 因此以 `TurnState::v2_executed` 拦下整段
//! `v2_facts::record`。但**派生事实不是轮次事实**，而是本轮的派生副作用——运行器
//! 一处都不写。本档因此在轮末直接调 bridge 档的两个写方（**同一份函数**，差别只在
//! 溯源锚：这里是原生写的 `u-{turn}` 格）：
//!
//! - [`super::v2_facts::record_learning`]：步 11 编码 / 步 12 检索锚 / 步 13 巩固 /
//!   步 22 技能观测与编译；
//! - [`super::v2_facts::record_derived`]：承诺（`commitment_events`，S08 §3）/
//!   任务表（`v2_tasks::write`，S7 步 16–18）/ 熔断（`CircuitBreaker::break_event`，
//!   S8 步 20）——三者的数据来源都在工具执行层（`Delegation` / `TaskDeclaration` /
//!   熔断理由），由 [`super::v2_tools::SessionDispatchPort`] 经 `take_derived` 交回。
//!
//! ## 本档不覆盖：写侧授权闸
//!
//! 只剩一类：**写侧授权闸**（`authorize_close`，
//! [plan/01 §7](../../../docs/plan/01-核心架构.md)）——bridge 档在落收束格**之前**判
//! `reply.first` / `reply.append`，运行器不判。补它要先决定运行器的落格路径怎么接闸，
//! 那是 core 侧的一处独立改动，不在本档的收束收尾里。
//!
//! 窗口：prompt 只带最近 `context_messages` 轮（含当前轮）——事实全量
//! 入格（append-only），**视图**才是窗口。
//!
//! 兜底语义：生成失败时运行器落 fallback 事件（I3）后，本函数把失败
//! **上抛**给 chat_loop（`Failed` 出口）——用户侧的失败呈现（错误状态 +
//! 重试）与 v1 保持一致；网格里已有兜底格，重试即新一轮（N3 靠构造成立）。
//!
//! 中止语义：生成被中止时运行器**不落收束格**（网格少一格是诚实缺口，
//! ADR-044 同源），本函数上抛 `Aborted`——chat_loop 走独立出口，会话结局
//! `aborted`（**不是** `failed`）。中止与失败在**类型上**分开（`AdapterError`
//! / `TurnOutcome.aborted`），不靠错误文本猜；两者出口因此可各自演化。
//!
//! 等待用户语义：工具报 `failure_kind = pending`（confirm / ask_user）时运行器
//! 停止工具循环且**不落收束格**（本轮还没了结），本函数按正常收束返回——
//! 待用户动作由分发方广播的 `user_prompt` 节点承载（`WaitingUserAction`），
//! 与 v1 的呈现一致。恢复（用户答完续跑同一轮）走 [`TurnResume`]，**不重开用户格**。
//!
//! 但**派生事实两幕都写**（等待幕与恢复幕都走到本函数轮末）：步 11 编码按**内容**
//! 去重（`RecallView::contains_content`）⇒ 同一轮不会编出第二条；恢复幕的召回视图
//! 非空（首幕刚编的那条）⇒ 会落一条 `memory.recalled`。两条的溯源锚都指原轮用户格
//! （I2：恢复不新开用户格）。判据 = e2e `t27`（`turnFacts` 数轮次事实、派生事实另数）。
//!
//! 与 bridge 档的**已知差异**（中止轮 / 等待轮）：bridge 档在收束时原子转写，
//! 中止即整轮不转写（网格零增长）；full 档必须先落用户格——prompt 从网格出，
//! 用户发言不入格就会从下一轮的 prompt 里消失。同一「诚实缺口」原则的两种落地，
//! 差别源于 prompt 来源（v1 读转写 / v2 读网格）。影响面：中止轮在 full 档
//! 计入兜底率的**分母**（用户格在网格里），bridge 档不计——分子两档都不计。

use std::sync::{Arc, Mutex};

use crate::symbio_core::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    llm_short_id, ActorSpec, CapabilityMeta, DeltaSink, DispatchPort, Entity, Event, EventWalStore,
    ExecAbortSignal, ExecEventSink, LatencyTier, ModelProvider, Plugin, PluginError,
    ProviderLlmAdapter, Seq, Store, TokenIssuer, TurnOutput, TurnToolCallInfo, Verb,
    EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

use super::chat_session::PersistentChatSession;
// 运行器一族**已下沉到本插件**（2026-10-09，见 `turn_runner.rs` 的头注）：它们不是 core 的
// 契约面，生产消费方只有本插件，按 README §4 四问第 1 问该住这里。
use super::turn_runner::{TurnInput, TurnResume, TurnRunner};

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
    /// 该流在事实网格 / 界面上的节点类型。
    fn msg_type(self) -> MessageType {
        match self {
            UiNodeKind::Text => MessageType::Text,
            UiNodeKind::Reasoning => MessageType::Reasoning,
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
    /// 工具调用快照（身份帧）：**原样透传**的 `ChatMessage`。
    ///
    /// 为什么不拆进 [`UiFrame::Snapshot`]：工具节点的 `name` / `tool_call_id` /
    /// 参数至今都是 model 插件转写面给出的事实，拆开再拼只会引入第二份形状；
    /// 分发方稍后以**同一 id** 定格与落库（`llm_build_tool_call_nodes` 的节点 id
    /// 就是转写面的节点 id），节点永远只有一张卡。参数窄增量**不走**本变体——
    /// 它走 [`UiFrame::Delta`]（按 id 合并的语义与正文增量共用一条）。
    Tool(Box<ChatMessage>),
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
    /// 正文节点 id：首片分配、此后稳定；`finalize_node` 交还并置空（下一轮另开）。
    node: Arc<Mutex<Option<String>>>,
    /// 推理节点 `(id, 累积正文)`：与正文同形，但**多带一份累积正文**。
    ///
    /// 为什么正文不需要累积而推理需要：正文的收束帧由调用方带全文
    /// （`LlmTurn::text`，见 [`super::v2_tools::SessionDispatchPort`]），而推理
    /// 正文**没有别的持有者**——`LlmTurn` 只装 `text` 与 `tool_calls`。
    /// 不在这里攒，收束帧就只能写「推理到此为止」而不带内容。
    ///
    /// 累积是无损的：每一片都先经这里再进队列，队列满了是**合并**同节点增量
    /// （见 `FrameQueue`），不丢。
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
    /// 当前正文节点 id（未定格的最后一轮正文）。
    pub(crate) fn current_node(&self) -> Option<String> {
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
        let id = self.node.lock().unwrap().take()?;
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
                    kind: UiNodeKind::Text,
                }
            }
        };
        // 有界队列：入队不阻塞（`on_delta` 是同步回调，不能 await），满了合并
        // 增量而不是丢——丢增量等于丢正文。见 `FrameQueue`。
        let _ = self.tx.send(frame);
    }

    /// 推理增量：与 [`Self::on_delta`] **同形**（首片建节点、此后窄追加），
    /// 只差两处——节点类型是 `Reasoning`，且正文要**累积**（见 `reasoning` 字段）。
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
    fn on_tool_frame(&self, m: &ChatMessage) {
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
    /// 本轮开头召回的长期记忆视图（S5 步 12）——轮末由
    /// [`super::v2_facts::record_learning`] 落一条 `memory.recalled`。
    ///
    /// 它**不是**轮次事实，因此不随 `v2_executed` 的拦截一起消失：v2 原生记账
    /// 只覆盖轮次事实，记忆与学习的写方两档共用同一个函数（见该函数文档）。
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
    /// 空表是**常态**（没开 `skill_fast_path` / 没编过技能 / 档位不是 `full`）——
    /// 那时本轮与接线前逐字同路（`run_with_tools`）。填充点唯一：
    /// `chat_loop/inputs.rs` 的 `fast_armed` 分支（与观测同一次 `route`）。
    pub skill_hits: &'a [super::v2_skills::SkillLlmHit],
    /// **请求级前缀**：v1 请求视图层 `build_request_view` 置顶的三段——长期记忆召回 /
    /// 就绪任务集 / 委派者真源——在这里拼成一段文本，排在本轮对话**之前**
    /// （见 [`TurnInput::prefix`](crate::plugins::session::turn_runner::TurnInput::prefix)）。
    ///
    /// ## 为什么必须显式带进来
    ///
    /// `full` 档的 prompt 是**一条** user 消息（`ProviderLlmAdapter::generate_turn`
    /// 把渲染结果整段发出），而那三段在 v1 里是**独立的
    /// 消息**。于是档位翻成 `full` 的同时它们**静默**从模型眼前消失——事实照样入格、
    /// `session/stats` 照样有数，只有模型看不见（实测 t29 / t35 在 `full` 下报
    /// 「记忆段没注入」，而注入逻辑一行没改）。
    ///
    /// ## `None` 是常态
    ///
    /// 三段都空的情形：首轮没有记忆、没有就绪任务、判定不出委派者。取法见
    /// `chat_loop` 的 `request_view_prefix`。
    pub prefix: Option<String>,
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
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatMessage>>>,
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
        .with_payload(serde_json::json!({ "text": text, "count": count })),
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
    let msg = ChatMessage {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        role: Some(MessageRole::User),
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

/// 静默增量口：定稿答话轮一个模型请求都不发 ⇒ 没有任何增量可送。
///
/// **为什么不用 core 的 `SilentDeltas`**：它不在根出口（core 内部自己走域内路径用），
/// 而 C-002 不许插件深引 `symbio_core::adapters::…`。为一次「什么都不做」去拓宽 core 的
/// 公开面（并因此多一个单消费方符号、动棘轮）不划算——插件侧 4 行更干净。
struct SilentDeltaSink;

impl DeltaSink for SilentDeltaSink {
    fn on_delta(&self, _text: &str) {}
    fn on_reasoning(&self, _text: &str) {}
    fn on_tool_frame(&self, _frame: &ChatMessage) {}
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
/// 补写就成了**第二个轮次事实写方**——同轮两份记账的假象随之回来（那正是
/// `chat_loop` 的 `!turn.v2_executed` 拦截当年要防的形状）。所以答话交给运行器
/// （`TurnInput::final_reply`），格仍由它落。
///
/// ## 与 [`execute_turn`] 的两处刻意差异
///
/// 1. **不写派生事实**：`record_learning` / `record_derived` 一律不调——`Answered` 轮没有
///    工具交换、没有模型参与，与 v1 那条路（`finish_turn` 直接收尾、不转写）逐字一致。
/// 2. **不上线 UI 帧**：答话节点已由 `apply_verdict` 推进 `context.messages`
///    （`dialog_node`，`exclude_from_context = false`），由 `finish_turn` 落库 ⇒ 这里用
///    [`SilentDeltaSink`]；再发一次就是同一条答话重复上屏。
///
/// ## `llm` / `tok` 是 `run_with_tools` 的端口，不是本路的依赖
///
/// 短路发生在**用到它们之前**（`TurnRunner::run_with_tools` 步骤 1b）——这一轮一次模型
/// 往返都不发。仍然照签名传，是因为「工具轮入口」只有那一个：另开一个入口就得把
/// 「用户格 + 收束格」的落格逻辑写第二遍，那才是真正的重复。
pub(crate) async fn execute_final_reply_turn(
    session: &PersistentChatSession,
    ctx: &Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    provider: Arc<dyn ModelProvider>,
    abort: ExecAbortSignal,
    user_text: &str,
    reply: &str,
) -> Result<(), PluginError> {
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("full 档需要持久会话（临时会话无事实源）".into())
    })?;
    let wal = dir.join(super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal)
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?;
    let snapshot = store.range(Seq::new(0));
    // turn 号口径与 [`execute_turn`] 同源（WAL 内既有 `user.message` 计数）——两处各写
    // 一套必然在「答话轮之后紧接着一个新轮」时错开一号。
    let turn_no = count_user_messages(&snapshot);
    // 身份与档位取值点同 [`execute_turn`]：主体 = 会话选定的那个 agent；档位 = 深度
    // （`Answered` 轮没有可跳过的模型调用，不属于反射档）。
    let principal = super::chat_loop::request_principal(ctx.as_ref());
    let actor = ActorSpec {
        budget_ms: LatencyTier::Deep.budget_ms(),
        ..ActorSpec::trivial(principal)
    };
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
    };
    TurnRunner
        .run_with_tools(
            &store,
            &llm,
            &tok,
            input,
            Arc::new(SilentDeltaSink),
            &[],
            None,
        )
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
    } = req;
    // 事实源：per-session v2 WAL（与桥同一个文件——两档共用一份网格）。
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("full 档需要持久会话（临时会话无事实源）".into())
    })?;
    let wal = dir.join(super::paths::V2_WAL_FILE);
    // `Arc` 是给轮内注入闭包用的（`TurnInjector` 是 `'static` 闭包，而
    // `WalStore` 不 `Clone`）。**只有它 clone**——`Arc` 共享同一份，注入的落格
    // 与本函数末尾的落格进的是同一个文件，append-only 下不会打架。
    let store = std::sync::Arc::new(
        EventWalStore::open(&wal)
            .map_err(|e| PluginError::InternalError(format!("v2 WAL 打开失败：{e}")))?,
    );
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

    // 折进的补充消息的收集器（缺口 3）：注入闭包（`'static`）往它 push，
    // 执行完随 `V2TurnResult` 回灌进 `context.messages`——**与 v1 路径同动作**
    //（那边是 `context.messages.push(merged)`）。不回灌就只活在这一次请求里，
    // 重启即丢，而 t19 等的正是落库那条。
    let collected: Arc<std::sync::Mutex<Vec<ChatMessage>>> =
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
    let emitter = tokio::spawn(async move {
        while let Some(frame) = tx.pop().await {
            let message = match frame {
                UiFrame::Snapshot {
                    id,
                    parent,
                    text,
                    kind,
                } => ChatMessage {
                    id,
                    parent_id: Some(parent),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(kind.msg_type()),
                    content: Some(MessageContent::Text(text)),
                    status: Some(MessageStatus::Streaming),
                    ..Default::default()
                },
                UiFrame::Delta { id, text } => ChatMessage {
                    id,
                    delta: Some(text),
                    ..Default::default()
                },
                UiFrame::Finalize {
                    id,
                    parent,
                    text,
                    kind,
                } => ChatMessage {
                    id,
                    parent_id: Some(parent),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(kind.msg_type()),
                    content: Some(MessageContent::Text(text)),
                    status: Some(MessageStatus::Completed),
                    ..Default::default()
                },
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
    let hit = resume_anchor
        .is_none()
        .then(|| super::v2_skills::take_match(skill_hits, user_text))
        .flatten();

    // 档位：命中 ⇒ 反射（`budget_ms = 80`、开轮格的 `tier = reflex`），未命中 ⇒ 深度。
    // `tier` 与 `actor.budget_ms` 是**同一个事实**（一个入格、一个随主体走），
    // 故由前者派生后者——两处各写一遍迟早对不上（S11 §3 的 ActorSpec 行）。
    //
    // **诚实缺口**：`ActorSpec.budget_ms` 在生产里**还没有读方**——`invariants` 的
    // `declared_budget_ms` 读的是开轮载荷里的 `tier` **字符串**（再映射回同一个数）。
    // 于是这里派生出的数是"算了却看不见"：不派生也不改变今天的行为（`trivial()` 的
    // `60_000` 同样无人读）。仍然派生的理由 = 它是 I3 预算的**正确值**，读方接进来时
    // 不必回头改这里；而读方该读 `tier` 还是该读这个字段（= 改事件载荷形状），
    // 是 `invariants` 那一侧的取舍，本模块两个都不替它定。
    let tier = if hit.is_some() {
        LatencyTier::Reflex
    } else {
        LatencyTier::Deep
    };
    // 身份进事件（[plan/11 批 1](../../../docs/plan/11-多执行器与多主体加固实施方案.md) ①）：
    // 本轮以**会话选定的那个 agent** 为主体——`ActorSpec` 首次在生产构造，
    // `agent:main` 从字面量变成数据。取值点与转写侧同一个（`request_principal`），
    // 写侧闸判的正是它 ⇒ 判的对象 = 写的对象。未选智能体 ⇒ `agent:main`
    // （S08 §4 平凡值，与接线前逐字一致）。
    //
    // 另存一份 `principal`：`actor` 随 `input` 被移动进运行器，轮末的记忆与学习
    // 写方（`record_learning`）仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    let principal = super::chat_loop::request_principal(ctx.as_ref());
    let actor = ActorSpec {
        budget_ms: tier.budget_ms(),
        ..ActorSpec::trivial(principal.clone())
    };

    let mut produced = Vec::new();
    // 工具执行层交回的收束派生事实（承诺 / 任务表 / 熔断）——只有工具轮（深度档）才有，
    // 反射档与无工具轮保持默认空值。轮末随收束落格（与 bridge 档同一写方，见下方）。
    let mut derived = super::v2_tools::DerivedFacts::default();
    let outcome = match &hit {
        // ── 反射档（S11 §2）：**没有 `llm` 形参**的那条路 ────────────────────
        // "反射档调模型"在类型上写不出来（`run_reflex` 只收 `RuleOnly`）——
        // 这正是 `docs/plan/verify/latency_gate.rs::assemble(Reflex)` 的落地形态
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
                            // 顺序：**先抽干、后汇报**（v1 同一条纪律，见 `chat_loop.rs`
                            // 步骤 2d）——抽干会把静默时钟归零，反过来就会出现
                            // 「用户刚说完话，助手抢着报了一句进度」。
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
                            if let Some(report) = report {
                                if let Some(draft) = report(anchor).await {
                                    match anchor {
                                        Some(anchor) => {
                                            let no = supplemental_no.fetch_add(
                                                1,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
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
                                                })),
                                            ) {
                                                crate::plugin_warn!(
                                                    "session",
                                                    "[Turn] 汇报落格失败（仍回灌落库镜像，下一轮起模型看不见）: {e:?}"
                                                );
                                            }
                                            if let Ok(mut sink) = sink_msgs.lock() {
                                                sink.push(draft.to_message());
                                            }
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
    let child_id = bridge.current_node().unwrap_or_default();
    let (reasoning_child_id, reasoning_text) = bridge.current_reasoning().unwrap_or_default();
    // 桥归零 ⇒ 通道关闭 ⇒ 发射端排空后退出。
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：运行器**未落收束格**（网格少一格是诚实缺口，ADR-044 同源），
    // 出口走 Aborted——由 chat_loop/消费循环落库为 `MessageStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`），与 v1 的中止出口同形。
    // 记忆与学习也**不写**：与 bridge 档的中止出口同形（那一档的 `TurnExit::Aborted`
    // 同样不转写）——中止的轮次没有收束，没有可固化的东西。
    if outcome.aborted {
        return Err(PluginError::Aborted);
    }

    // ── 收束派生事实（两半）：承诺 / 任务表 / 熔断 + 记忆与学习 ────────────────
    //
    // **本步的存在理由**：`full` 档的轮次事实由运行器原生入格，chat_loop 因此以
    // `v2_executed` 拦下整段 `v2_facts::record`。但这两半都**不是轮次事实**，而是
    // 本轮的**派生副作用**——运行器一处都不写。不在这里补，full 档的代际立约（S08）/
    // 任务表（S7）/ 熔断（S9 §6 验收 2）/ 长期记忆（S06）/ 技能自我改进（S11）就整体
    // 失效，而档位名还自称「整体切换」。写方与 bridge 档**同一份函数**
    // （`record_derived` / `record_learning`），差别只在锚点：这里是 v2 原生写的
    // `u-{turn}` 格，那里是转写的 `v2u-*` 格。
    //
    // 兜底收束的 `response` 为 `None`：兜底说明这条路没走通，固化它等于把失败写成
    // 套路（与 bridge 档 `V2Closure::Fallback` → `success_text = None` 同一条口径）。
    // 写侧闸拒（`closure_denied`，见 core 的 `closure_granted`）：收束格**未入格**
    // ⇒ 本轮的派生事实**也不写**——与 bridge 档 `authorize_close` 拒绝时同形（那一档的
    // `record_to_wal` 在闸处提前返回，承诺 / 任务 / 熔断 / 记忆全不写）。用户的答案
    // 照旧经 `produced` 回给调用方——收束格是**记录**，不是呈现。
    if outcome.closure_denied {
        crate::plugin_warn!(
            "session",
            "[v2-exec] 收束被授权拒绝，本轮派生事实不入格（turn={turn_no}）"
        );
        // 被拒的兜底仍是**失败**（I3）：呈现走 v1 的 Failed 出口，与未拒时同形。
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
                ..Default::default()
            },
            messages: produced,
        });
    }

    let response = (!outcome.fell_back).then_some(outcome.text.as_str());
    match store
        .range(Seq::new(0))
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE && e.turn == turn_no)
        .and_then(|e| e.seq.map(|s| s.value()))
    {
        Some(user_seq) => {
            // 承诺 / 任务表 / 熔断：与 bridge 档**同一写方**（`record_derived`）。
            // 承诺失败只记日志不冒泡——轮次已收束，派生事实失败不该把成功的一轮说成
            // 失败（与记忆三段同一条口径）。
            if let Err(why) = super::v2_facts::record_derived(
                &wal,
                &store,
                &store.range(Seq::new(0)),
                turn_no,
                user_seq,
                &principal,
                &format!("t{turn_no}"),
                &derived.delegations,
                &derived.tasks,
                &derived.breaks,
            ) {
                crate::plugin_warn!("session", "[v2-exec] 承诺入格失败（turn={turn_no}）：{why}");
            }
            // 记忆与学习：同一函数（`record_learning`）。
            super::v2_facts::record_learning(
                &wal,
                // `store` 是 `Arc<WalStore>`（轮内注入闭包要 `'static`），而本函数
                // 收 `&WalStore` ⇒ 要一次显式 deref。写 `store.as_ref()` 而不是
                // `&*store`：后者会被 clippy 判 `needless_borrow`（它建议 auto-deref，
                // 但 auto-deref 到 `Arc` 本身，而 `Arc<WalStore>` 不实现 `Store`）。
                store.as_ref(),
                &store.range(Seq::new(0)),
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

    // 兜底已入格（I3），失败呈现交回 v1 语义（Failed 出口 + 重试）。
    if outcome.fell_back {
        return Err(PluginError::InternalError(outcome.text));
    }
    // 等待用户（`outcome.awaits_user`）：本轮尚未了结（运行器未落收束格），但
    // **按正常收束返回**——待用户动作由分发方广播的 `user_prompt` 节点承载
    // （`WaitingUserAction`），v1 的呈现（卡片 + 恢复入口）不变；恢复时经
    // [`TurnResume`] 续写同一轮（不重开用户格）。此处的 `text` 只是模型在工具轮说过的话，不是终答。

    // 折进的补充也要进落库镜像（缺口 3）：排在 `produced` **之前** ——
    // 用户先插话、工具结果后到，这是时间序（`persist_messages` 只写尾部增量，
    // 顺序错了落库就乱了）。
    // 同上：锁拿不到就用空的（落库镜像少这批补充，其余不受影响）。
    let mut all_messages: Vec<ChatMessage> = collected
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
            ..Default::default()
        },
        messages: all_messages,
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
