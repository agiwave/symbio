use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use symbio_kernel::exec::ExecAbortSignal;
use symbio_sdk::chat::{chat_node as cm, session_chat};
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;
use tokio::sync::{Mutex, Notify, RwLock};

use super::plugin::SessionRuntime;
use super::runtime::RuntimeFacts;
use super::working::{WorkingState, WorkingTransition};

/// 收件箱里的一条**待消费请求**——用户消息的入队形态。
///
/// ## 它不是一条已在转写里的消息
///
/// 条目存在期间，这条消息**还没有落库**（也不该落库：它还没被消费）。被消费时
/// 才由统一的发送链路追加进存储，随即出现在转写里。于是「收件箱」与「转写」是
/// 同一批消息的**前后两态**，不是两份需要对齐的数据。
///
/// ## 为什么把请求拆成 `message` + `params` 两半
///
/// 入队必须与直连发送**信息等价**，否则「经由收件箱发送」会悄悄丢掉本次运行的
/// 请求级参数（`mode` / `risk_level` / `agent_id` / `provider_id` / `include_history`）
/// ——这种丢失是静默的，只在行为上表现为"选项不生效"。
///
/// 于是 `params` 就是请求里**与"哪条消息发给哪个会话"无关**的那一半：
/// `session_id` 由地址给出（`<sid>/inbox`），`message` 就是 [`Self::message`]，
/// `resume` 恒为 `None`（恢复不是"一条用户消息"，它不走收件箱）。三者都不在
/// `params` 里——同一件事只有一个位置，避免两处各存一份而要互相校验。
///
/// ## 为什么**不存**发起者的请求上下文
///
/// 消费者自己造一个干净上下文（只带目标会话 id 与工作目录），不复用发起者的
/// `Arc<dyn PluginInvokeRequest>`：发起者的头里带着**它自己**的 `SESSION_ID`，而跨空间
/// 写入（父会话 → 子智能体空间）时那个 id 与目标会话**不是同一个**——沿用它会
/// 让消费者把消息投到发起者的会话上。工作目录因此单独存一份（它确实是本次请求的
/// 信息，且不能从目标会话的 metadata 必然推出）。
#[derive(Clone)]
pub struct InboxItem {
    /// 条目 id = VDFS 地址末段 = 这条用户消息的 `ChatNode.id`
    pub id: String,
    /// 待消费的用户消息
    pub message: cm::ChatNode,
    /// 请求级参数（见上：不含 `session_id` / `message` / `resume`）
    pub params: session_chat::Request,
    /// 发起者给出的工作目录（请求级，优先于会话 metadata 的回退值）
    pub workdir: Option<String>,
}

/// 抢占判定的**待结算项**（[DE-016 §9.2](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
///
/// 判定与落格必须分开，因为两者要的时刻不同：
/// - **判定**只能在忙窗（`is_working == true` 且有插话排着）做——那时事实源里只有
///   上一轮的收束入格，而写方（v2 运行器的轮末收束）还在等这一轮收尾，并发 append 会
///   重复 `head`（同会话不许两个写方并存）；
/// - **落格**只能在空闲分支做——`task.held` 要在插话轮**开跑之前**就位，
///   `task.progress`（恢复）要等插话轮**结束之后**再写。
///
/// 中间这段时间由本项背着跨过去，所以它住在会话状态里而不是栈上。
///
/// 三态而非两态：`Hold` 是"判出来了但还没落格"，`Resume` 是"落了格等收"。
/// 少一态就会把"待收"和"未判"混为一谈——那会让挂起在插话轮结束后永久留存，
/// 任务从此不进 `readyset`，且没有任何一条消息说得出为什么（DE-037 §5 的静默失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreemptPending {
    /// 无事待办（未判过，或已结清）。
    None,
    /// 判出挂起，尚未写入事实源。
    Hold {
        /// 被挂起的存活任务节点。
        task_id: String,
        /// 挂起锚（任务开格 seq，随事实一起落盘，见 `held_event`）。
        as_of_seq: u64,
    },
    /// 挂起已落格，等插话轮结束写恢复事件。
    Resume { task_id: String },
}

/// 会话内部状态 (封装为单个锁定对象以保证原子性)
pub struct ActiveSessionStateInner {
    /// 工作态（进程局部量，[`WorkingState`]）。
    ///
    /// **字段私有**：生产侧的写点只有 `ActiveSessionState` 上的迁移方法
    /// （`begin_turn` / `finish_turn`），它们都经 [`WorkingState::apply`] 并把
    /// 「由忙变闲」交给 `turn_done` 唤醒。收件箱消费者因此不再轮询。
    working: WorkingState,
    /// 在途 Turn 的中止信号（登记后中止动作直接置位，无需投帧）。
    ///
    /// 收口前这里是 `Option<mpsc::Sender<PluginFrame>>`——中止只能靠「往执行期
    /// 通道投一帧 `ControlSignal::Abort`」，执行方在 `select!` 里收帧再置位标志。
    /// 现在登记的就是那个信号本身：置位即生效、置位即唤醒（无轮询、无帧）。
    ///
    /// **唯一所有者是 [`ActiveSessionState`] 上的 `abort_signal` 格子**：事实源
    /// 只持同一个 `Arc` 的引用，不另存一份 `Option`（两份就是两个真源）。
    pub last_content: String,
    pub last_tool_calls: Vec<Value>,
    /// 上一轮交互的**结局**（`completed` / `aborted` / `failed`）。
    ///
    /// 这是**状态**而不是事件：提示音 / 错误条 / 重试入口都据此判定，
    /// 不需要「Abort 与 Error 谁先到」这种顺序知识（见
    /// `docs/105-node-state-streaming.md` §3.3）。
    ///
    /// `None` = 本进程内还没有跑完过一轮（或刚被新一轮请求复位）。
    pub last_outcome: Option<String>,
    /// 上一轮的错误短消息（面向用户）。仅当 `last_outcome == failed` 时存在。
    ///
    /// 存在的意义：错误可能发生在**任何消息节点创建之前**（能力收集失败 /
    /// provider 解析失败 / transport 级失败），此时没有失败节点可承载它，
    /// 只能挂在会话节点上——原先是前端一个平行状态（`sessionErrors`），
    /// 现在它是节点的属性（`attributes.error`）。
    pub last_error: Option<String>,
    /// **收件箱**：待消费的用户请求队列（FIFO）。
    ///
    /// 它是「用户消息」的唯一入队形态（见 [`InboxItem`]），也是 VDFS 集合
    /// `<sid>/inbox` 的数据源。条目**出队即消失**：被消费时消息由统一发送链路
    /// 落库并出现在转写里，队列里因此只留"还没轮到"的那些——
    /// 「正在处理的那条」在转写里，不在队列里（取消它走中止，不走删条目）。
    pub inbox: VecDeque<InboxItem>,
    /// **一忙窗一判**的标记（见 [`PreemptPending`]）：`task.*` 与
    /// `chat.assistant.final` 都只在轮末收束时入格，
    /// 忙窗内事实源不会长出新的判据，所以判过一次就够了——留个标记免得每一趟醒来
    /// 都重放一遍整条 WAL。空闲分支复位它，下一轮忙窗重新判。
    pub preempt_judged: bool,
    /// 抢占判定的待结算项（见 [`PreemptPending`]）。
    pub preempt: PreemptPending,
}

impl ActiveSessionStateInner {
    /// 是否在途忙——`session_busy` 守卫、收件箱「忙则排队」、运行态投影共用的**唯一**读法。
    pub(crate) fn is_working(&self) -> bool {
        self.working.is_working()
    }

    /// 测试专用：直接摆位工作态。
    ///
    /// 生产侧故意没有这个口子——改变工作态只能经三个迁移方法，这样「变了但没人
    /// 被叫醒」在结构上不可能发生。测试要造「忙窗」时没有历史轮可收，才用它。
    #[cfg(test)]
    pub(crate) fn set_working_for_test(&mut self, working: bool) {
        self.working = if working {
            WorkingState::Working
        } else {
            WorkingState::Idle
        };
    }
}

/// 会话状态锚点
pub struct ActiveSessionState {
    /// 会话 ID 字符串（用于 EventBus 标签）
    pub session_id: String,
    /// 在途 Turn 的中止信号格：**唯一所有者**。
    ///
    /// 事实源（`SessionFactSource`）只持同一个 `Arc` 的引用，每次踢驱动时读它的
    /// **当前值**挂进 `DriveTarget`；中止动作也只置位、不取走——取走之后本轮后续
    /// 派发读到的是新造的死信号，中止就退化成「只打断当前这次调用」。
    /// 注销只在 [`ActiveSessionState::finish_turn`] 的同一临界区做一次。
    ///
    /// 用同步锁：事实源的 `append` 是同步函数，不能 await。
    pub(crate) abort_signal: Arc<std::sync::RwLock<Option<ExecAbortSignal>>>,
    pub inner: RwLock<ActiveSessionStateInner>,
    /// 会话**转写**（在途图 + 单调 seq + 实时发布）。
    ///
    /// ## 一份数据，两个视图
    ///
    /// 这是消费循环里唯一写入的那个 Transcript（见 `transcript.rs`），不是第二份拷贝：
    ///
    /// - 实时面：每条变更（`created` / `updated` + `delta` / `deleted`）经
    ///   `apply` 投到 VDFS 变更订阅表——一张表、一条 `vdfs/watch`；
    /// - VDFS 转写列表：`<根>/session/<id>/message` 把在途图叠加在落库转写之上。
    ///
    /// 之所以必须共享：**流式期间消息还没落库**（`persist_messages` 只在每轮结束时
    /// 写盘）。若 VDFS 只读存储，列表在流式期间就是空的。
    ///
    /// 落库后不摘节点：读出口 `overlay_live` 按 id 合并存储副本与在途副本，
    /// 同一条消息只出一条（权威号保留存储那一侧）。
    pub transcript: Arc<Mutex<super::transcript::Transcript>>,
    /// 「本轮收尾」唤醒：
    /// 工作态**由忙变闲**时通知一次。
    ///
    /// 收件箱消费者挂在它上面等「某个排队会话让出」，取代此前的 50ms 轮询。
    /// 用 `notify_one` 而不是 `notify_waiters`：前者会给**尚未挂起**的等待者留一个
    /// 许可，而「收轮恰好发生在消费者重新挂起之前」正是轮询当初要兜的那个窗口。
    pub(crate) turn_done: Arc<Notify>,
}

impl Default for ActiveSessionState {
    fn default() -> Self {
        Self::new(VdfsChangeSubscriptions::default())
    }
}

impl ActiveSessionState {
    /// `changes` 是**本 provider 自持的那张 VDFS 变更表**——转写的实时面投给它。
    /// 不收它、让 `Transcript` 自己去取全局表，会投到没有订阅者的那张表上
    /// （见 `Transcript::new` 的说明）。
    pub fn new(changes: VdfsChangeSubscriptions) -> Self {
        Self::with_session_id(String::new(), changes)
    }

    /// 带 session_id 的构造函数
    pub fn with_session_id(session_id: String, changes: VdfsChangeSubscriptions) -> Self {
        Self {
            session_id: session_id.clone(),
            abort_signal: Arc::new(std::sync::RwLock::new(None)),
            inner: RwLock::new(ActiveSessionStateInner {
                working: WorkingState::Idle,
                last_content: String::new(),
                last_tool_calls: Vec::new(),
                last_outcome: None,
                last_error: None,
                inbox: VecDeque::new(),
                preempt_judged: false,
                preempt: PreemptPending::None,
            }),
            transcript: Arc::new(Mutex::new(super::transcript::Transcript::new(
                session_id,
                Arc::new(changes),
            ))),
            turn_done: Arc::new(Notify::new()),
        }
    }

    /// 工作态**由忙变闲**时叫醒收件箱消费者（一次迁移至多一次唤醒）。
    fn wake_if_idle(&self, became_idle: bool) {
        if became_idle {
            self.turn_done.notify_one();
        }
    }

    // ==================== 运行态的写路径（单写者）====================
    //
    // `is_working` / `last_outcome` / `last_error` 的**唯一写点**。
    // 它们不是第二份可独立演化的状态：结局是 turn 事实投影（`runtime::project`）的
    // 写穿缓存，开轮 / 收轮 / 中止各只经一个方法（[D-083 §5.2]）。
    // 拆散成调用点各写各的字段，正是要消掉的双写面。
    //
    // 工作态的迁移是**闭集**：只有 `WorkingTransition` 的三个变体，且唯一迁移函数是
    // `WorkingState::apply`。新增收尾路径若要改工作态，只能经这里——绕过它状态不会变，
    // 而不是「变了但收件箱没人被叫醒」（那正是 50ms 轮询掩盖过的失效）。

    /// 开轮：置忙 + 上一轮结局作废 + 清空上一轮的流式累积。
    ///
    /// 忙碌位必须在 `spawn` **之前**同步可见——并发排队（t15/t32/t34）的守卫
    /// 读的就是它，而开轮事实是 spawn 之后才追加的。
    pub(crate) async fn begin_turn(&self) {
        let mut inner = self.inner.write().await;
        inner.working.apply(WorkingTransition::Begin);
        inner.last_content.clear();
        inner.last_tool_calls.clear();
        inner.last_outcome = None;
        inner.last_error = None;
    }

    /// 收轮：结局与错误文案（`failed` 时才带）落进缓存。
    ///
    /// 正常 / 中止 / 失败三条出口写同一个值域；事实侧的同一结局由运行器 / 失败收口
    /// 落格，重启后由投影补上。
    pub(crate) async fn finish_turn(&self, outcome: &str, error: Option<String>) {
        let became_idle = {
            let mut inner = self.inner.write().await;
            // 同一临界区：工作态落闲与注销中止信号一起可见——不留「已收轮但信号
            // 仍置位」的窗口（那会让下一轮带着上一轮已取消的信号开局）。
            *self
                .abort_signal
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            let became_idle = inner.working.apply(WorkingTransition::Finish);
            inner.last_outcome = Some(outcome.to_string());
            inner.last_error = error;
            became_idle
        };
        self.wake_if_idle(became_idle);
    }

    /// 开轮登记：把**本轮唯一的信号对象**放进格子。必须在第一次 `append` 之前调用
    /// ——第一次 append 就会踢驱动，而 `DriveTarget` 只带当时的当前值。
    pub(crate) fn register_abort_signal(&self, signal: ExecAbortSignal) {
        *self
            .abort_signal
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(signal);
    }

    /// 中止在途一轮：**只置位、不取走**。返回是否确有在途信号（没有 ⇒ 会话没在跑）。
    ///
    /// 取走会让本轮后续派发（如工具行）读到空槽、由环现造一个死信号——那正是
    /// 「用户按下中止、工具照常跑」。注销由 [`Self::finish_turn`] 在同一临界区做。
    pub(crate) fn abort_current_turn(&self) -> bool {
        let slot = self
            .abort_signal
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.as_ref() {
            Some(signal) => {
                signal.abort();
                true
            }
            None => false,
        }
    }
}

/// 无进程内缓存时的运行态读法：**事实投影**作答。
fn runtime_from_facts(facts: Option<&RuntimeFacts>) -> SessionRuntime {
    match facts.and_then(|f| f.outcome.clone()) {
        Some(outcome) => {
            let (outcome, error) = outcome.into_state();
            SessionRuntime::finished(&outcome, error)
        }
        None => SessionRuntime::idle(),
    }
}

/// 合并规则：**工作态压过一切；有缓存结局以缓存为准；否则由投影作答**。
///
/// 缓存领先事实是正常路径（中止先写内存、收束格随后落盘），所以不能反过来让
/// 旧事实覆盖新缓存；而没有缓存时（重启 / 从未在本进程收过轮）事实就是唯一来源。
fn runtime_of_state(
    inner: &ActiveSessionStateInner,
    facts: Option<&RuntimeFacts>,
) -> SessionRuntime {
    if inner.is_working() {
        return SessionRuntime::working();
    }
    match &inner.last_outcome {
        Some(outcome) => SessionRuntime::finished(outcome, inner.last_error.clone()),
        None => runtime_from_facts(facts),
    }
}

/// 活跃会话管理器
pub struct ActiveSessionManager {
    pub sessions: Arc<RwLock<HashMap<String, Arc<ActiveSessionState>>>>,
    /// VDFS 变更表句柄：**每一份**新建的 `ActiveSessionState` 都把自己的
    /// `Transcript` 接到这张表上。共享句柄而非各建一张——`vdfs/watch` 登记的是
    /// 插件那一张（`SessionPlugin::change_subs`），别的表上没有订阅者。
    changes: VdfsChangeSubscriptions,
}

impl ActiveSessionManager {
    pub fn new(changes: VdfsChangeSubscriptions) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            changes,
        }
    }

    pub async fn get_or_create(&self, session_id: &str) -> Arc<ActiveSessionState> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get(session_id) {
            return state.clone();
        }
        let state = Arc::new(ActiveSessionState::with_session_id(
            session_id.to_string(),
            self.changes.clone(),
        ));
        sessions.insert(session_id.to_string(), state.clone());
        state
    }

    /// 单个会话的**运行态**（节点状态的唯一来源，见 [`SessionRuntime`]）。
    ///
    /// 住在名册上是因为它读的就是这张 `sessions` 表。读法分两层：
    /// - **有活跃状态**：读进程内缓存（`working` 压过一切；结局以缓存为准——
    ///   它可能早于收束格落盘，见 `begin_turn` / `finish_turn`）；
    /// - **无活跃状态**（重启 / 本进程从未跑过该会话）：由 `facts` 投影作答，
    ///   于是上次终态在重建面仍可见（SC-005）。
    ///
    /// 三处读路径（`list` / `stat` / 变更发射）都经这里，口径因此只有一份。
    pub(crate) async fn runtime_of(
        &self,
        id: &str,
        facts: Option<&RuntimeFacts>,
    ) -> SessionRuntime {
        let active = self.sessions.read().await;
        let Some(st) = active.get(id) else {
            return runtime_from_facts(facts);
        };
        // `try_read` 失败（正被写者持有）时按"运行中"回答：运行态的写入都是
        // 瞬时操作，读不到就说明此刻正在变更——宁可多报一次运行中（前端会
        // 在下一次变更收敛），也不要凭空把正在跑的会话报成空闲（那会让停止
        // 按钮消失、用户无法中止）。
        let Ok(inner) = st.inner.try_read() else {
            return SessionRuntime::working();
        };
        runtime_of_state(&inner, facts)
    }

    // ==================== 转写发布（消息变更的唯一出口）====================
    //
    // 一切消息级变更（前端 CRUD 动作 / 用户消息定稿 / 失败与中止的终态收敛）
    // 都经 `Transcript::apply` 发布——内存图、seq、核心日志、发布在同一个函数里
    // 完成。VDFS 转写列表的在途叠加读的是同一份内存图，「存储一份 + 在途一份」
    // 两种表示从此同源。
    //
    // 这两个函数住在**名册**上而不是插件上，是因为「按会话 id 解析到它的那份转写」
    // 只需要这张 `sessions` 表——插件在这里没有别的贡献。

    /// 向会话转写发布一条消息帧（唯一出口）。
    pub(crate) async fn transcript_apply(&self, session_id: &str, message: cm::ChatNode) {
        let state = self.get_or_create(session_id).await;
        state.transcript.lock().await.apply(message);
    }

    /// 向会话转写发布一批消息帧（同一把锁内顺序应用，保持发布顺序）。
    pub(crate) async fn transcript_apply_all(&self, session_id: &str, messages: Vec<cm::ChatNode>) {
        let state = self.get_or_create(session_id).await;
        let mut tr = state.transcript.lock().await;
        for message in messages {
            tr.apply(message);
        }
    }
}

#[cfg(test)]
#[path = "active.test.rs"]
mod tests;
