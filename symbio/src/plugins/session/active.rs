use crate::symbio_core::ExecAbortSignal;
use crate::symbio_core::VdfsChangeSubscriptions;
use crate::symbio_core::{chat_node as cm, session_chat};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use tokio::sync::{Mutex, Notify, RwLock};

use super::plugin::SessionRuntime;
use super::runtime::RuntimeFacts;
use super::working::{WorkingState, WorkingTransition};

/// 全局请求 ID 生成器
pub static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

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

/// 抢占判定的**待结算项**（S8 第 19 步，[plan/04 §2.2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
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
/// 任务从此不进 `readyset`，且没有任何一条消息说得出为什么（S07 §5 的静默失效）。
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
    /// **字段私有**：生产侧的写点只有 `ActiveSessionState` 上那三个迁移方法
    /// （`begin_turn` / `reset_working` 与 `clear_working_and_abort_signal` /
    /// `finish_turn`），它们都经 [`WorkingState::apply`] 并把「由忙变闲」交给
    /// `turn_done` 唤醒。收件箱消费者因此不再轮询。
    working: WorkingState,
    /// 在途 Turn 的中止信号（登记后 `handle_abort` 直接置位，无需投帧）。
    ///
    /// 收口前这里是 `Option<mpsc::Sender<PluginFrame>>`——中止只能靠「往执行期
    /// 通道投一帧 `ControlSignal::Abort`」，执行方在 `select!` 里收帧再置位标志。
    /// 现在登记的就是那个信号本身：置位即生效、置位即唤醒（无轮询、无帧）。
    ///
    /// `None` 的双重含义与收口前一致：既表示「当前没有在途 Turn」，也是
    /// `handle_abort` 判断 chat_loop 是否已收敛的判据。
    pub abort_signal: Option<ExecAbortSignal>,
    pub last_content: String,
    pub last_tool_calls: Vec<Value>,
    /// 上一轮交互的**结局**（`completed` / `aborted` / `failed`）。
    ///
    /// 这是**状态**而不是事件：提示音 / 错误条 / 重试入口都据此判定，
    /// 不需要「Abort 与 Error 谁先到」这种顺序知识（见
    /// `session/docs/105-node-state-streaming.md` §3.3）。
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
    /// 会话级告警（可恢复，面向用户）：持久化失败 / 长度截断。
    ///
    /// 它是**状态**不是事件：由出口的告警通道（`ExecEventSink::warn`）写入、随会话节点
    /// `attributes.warning` 下发，前端按状态渲染；新一轮请求开始（`Working`）时清除。
    /// 与 `last_error`（失败终态）不同：告警不改变运行态，会话照常运行。
    pub last_warning: Option<String>,
    /// 连续压缩失败次数（自动路径与用户重试走同一条流水线，计数共用）。
    ///
    /// 熔断依据：LLM 压缩是**整段历史的完整请求**，失败一次就是数分钟的注定浪费。
    /// 连续失败达到阈值即"开闸"跳过自动压缩——历史已回滚、本轮仍能正常回复，
    /// 不必每轮再白等一次注定失败的请求（实测会话 `09d74431` 就是这种反复失败）。
    /// 任一次压缩**成功**清零；开闸后按冷却时长重试一次（半开），让瞬时错误自愈。
    pub auto_compress_failures: u32,
    /// 熔断开闸时刻（`None` = 未开闸）。用于冷却判断。
    pub auto_compress_circuit_opened_at: Option<std::time::Instant>,
    /// **永久失败标志**：历史已超出模型上下文上限（`input_over_limit`）。
    ///
    /// 与瞬时失败不同——历史只增不减，这类失败**注定复现**，重试没有意义；
    /// 熔断的半开重试对它必然再失败。置位后自动压缩一律跳过，直到下一次
    /// 压缩**成功**（例如用户手动清理历史后）才清除。
    pub auto_compress_over_limit: bool,
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
    pub request_id: AtomicU64,
    /// 会话 ID 字符串（用于 EventBus 标签）
    pub session_id: String,
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
    /// 生命周期：每轮开始时清空（防上一轮残留），本轮落库后清空（防与落库版本重复）。
    pub transcript: Arc<Mutex<super::transcript::Transcript>>,
    /// 「本轮收尾」唤醒（相应批次）：
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
            request_id: AtomicU64::new(0),
            session_id: session_id.clone(),
            inner: RwLock::new(ActiveSessionStateInner {
                working: WorkingState::Idle,
                abort_signal: None,
                last_content: String::new(),
                last_tool_calls: Vec::new(),
                last_outcome: None,
                last_error: None,
                last_warning: None,
                auto_compress_failures: 0,
                auto_compress_circuit_opened_at: None,
                auto_compress_over_limit: false,
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

    /// **提前复位** `is_working`（不发变更、不动运行态的其余字段）。
    ///
    /// 忙位的写入只经下文「三条迁移方法」：本方法供「先复位、后收尾」的路径使用——
    /// 收尾还要做几件事（清在途图 / 注销中止信号 / `persist_failure`），这几步期间
    /// 必须已经不处于 `is_working`，否则并发进来的 `chat/send` 会被 `session_busy`
    /// 守卫静默拒绝（用户点重试无任何反应）。
    ///
    /// 幂等：已是空闲时不写、也**不**唤醒等待方（见 [`WorkingTransition::EarlyExit`]）。
    /// 复位**不带**变更投递——那一步由收尾处的 `emit_session_state`（经 `finish_turn`）
    /// 完成，两者合起来才是「运行态收敛」。
    pub async fn reset_working(&self) {
        let became_idle = self
            .inner
            .write()
            .await
            .working
            .apply(WorkingTransition::EarlyExit);
        self.wake_if_idle(became_idle);
    }

    /// 工作态**由忙变闲**时叫醒收件箱消费者（一次迁移至多一次唤醒）。
    fn wake_if_idle(&self, became_idle: bool) {
        if became_idle {
            self.turn_done.notify_one();
        }
    }

    // ==================== 运行态的写路径（单写者）====================
    //
    // `is_working` / `last_outcome` / `last_error` / `last_warning` 的**唯一写点**。
    // 它们不是第二份可独立演化的状态：结局是 turn 事实投影（`runtime::project`）的
    // 写穿缓存，开轮 / 收轮 / 告警 / 中止各只经一个方法（[D-083 §5.2]）。
    // 拆散成调用点各写各的字段，正是 P5 要消掉的双写面。
    //
    // 工作态的迁移是**闭集**：只有 `WorkingTransition` 的三个变体，且唯一迁移函数是
    // `WorkingState::apply`。新增收尾路径若要改工作态，只能经这里——绕过它状态不会变，
    // 而不是「变了但收件箱没人被叫醒」（那正是 50ms 轮询掩盖过的失效）。

    /// 开轮：置忙 + 上一轮结局 / 告警作废 + 清空上一轮的流式累积。
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
        inner.last_warning = None;
    }

    /// 收轮：结局与错误文案（`failed` 时才带）落进缓存。
    ///
    /// 正常 / 中止 / 失败三条出口写同一个值域；事实侧的同一结局由运行器 / 失败收口
    /// 落格，重启后由投影补上。
    pub(crate) async fn finish_turn(&self, outcome: &str, error: Option<String>) {
        let became_idle = {
            let mut inner = self.inner.write().await;
            let became_idle = inner.working.apply(WorkingTransition::Finish);
            inner.last_outcome = Some(outcome.to_string());
            inner.last_error = error;
            became_idle
        };
        self.wake_if_idle(became_idle);
    }

    /// 会话级告警（可恢复）：只写自己的格，不改变运行态。
    pub(crate) async fn set_warning(&self, warning: Option<String>) {
        self.inner.write().await.last_warning = warning;
    }

    /// 中止的强制收口：置空闲 **并**注销中止信号——同一临界区，两格一起可见。
    pub(crate) async fn clear_working_and_abort_signal(&self) {
        let became_idle = {
            let mut inner = self.inner.write().await;
            let became_idle = inner.working.apply(WorkingTransition::EarlyExit);
            inner.abort_signal = None;
            became_idle
        };
        self.wake_if_idle(became_idle);
    }

    /// 自动压缩熔断阈值：连续失败达到此数即开闸跳过。
    pub const COMPRESS_CIRCUIT_LIMIT: u32 = 3;
    /// 开闸后的冷却时长：期间跳过自动压缩；冷却结束允许一次重试（半开）。
    pub const COMPRESS_CIRCUIT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

    /// 自动压缩熔断：是否应跳过本次自动压缩。
    ///
    /// 判定：连续失败达到阈值即认为"这次 LLM 压缩注定失败"——继续重试只是每轮
    /// 白等数分钟。开闸后冷却期内一律跳过；冷却结束放行一次（半开），由这次
    /// 成功与否决定是否复位或重开。
    ///
    /// **永久失败优先**：历史超限（`input_over_limit`）不参与半开重试——它不是
    /// 瞬时错误，冷却到期后的重试注定再失败（实测会话 `09d74431` 冷却到期后
    /// 每轮都白跑一次），故该标志置位期间一律跳过。
    pub async fn compression_should_skip(&self) -> bool {
        let inner = self.inner.read().await;
        if inner.auto_compress_over_limit {
            return true;
        }
        if inner.auto_compress_failures < Self::COMPRESS_CIRCUIT_LIMIT {
            return false;
        }
        match inner.auto_compress_circuit_opened_at {
            // 已开闸且冷却未到：跳过
            Some(opened) => opened.elapsed() < Self::COMPRESS_CIRCUIT_COOLDOWN,
            // 计数达标却没记录开闸时刻（不应发生）：保守放行一次探查
            None => false,
        }
    }

    /// 记录一次自动压缩**成功**：清零失败计数、开闸时刻与永久失败标志。
    /// 任何路径的压缩成功都调用——熔断只在"连续失败"时维持。
    pub async fn compression_record_success(&self) {
        let mut inner = self.inner.write().await;
        inner.auto_compress_failures = 0;
        inner.auto_compress_circuit_opened_at = None;
        inner.auto_compress_over_limit = false;
    }

    /// 记录一次自动压缩**失败**：计数 +1；首次达到阈值时记下开闸时刻。
    /// 冷却期内失败不刷新开闸时刻（避免「每次失败都重置冷却」导致永不重试）。
    ///
    /// `permanent`：是否永久失败（`input_over_limit`——历史只增不减，重试注定
    /// 复现）。为真时置 [`ActiveSessionStateInner::auto_compress_over_limit`]，
    /// 此后跳过不再有半开重试。
    pub async fn compression_record_failure(&self, permanent: bool) {
        let mut inner = self.inner.write().await;
        if permanent {
            inner.auto_compress_over_limit = true;
        }
        inner.auto_compress_failures += 1;
        if inner.auto_compress_failures >= Self::COMPRESS_CIRCUIT_LIMIT
            && inner.auto_compress_circuit_opened_at.is_none()
        {
            inner.auto_compress_circuit_opened_at = Some(std::time::Instant::now());
        }
    }

    /// 返回 session_id 字符串的便捷方法
    pub fn request_id_str(&self) -> String {
        if !self.session_id.is_empty() {
            return self.session_id.clone();
        }
        // 兜底：使用内部 request_id
        self.request_id.load(Ordering::SeqCst).to_string()
    }
}

/// 无进程内缓存时的运行态读法：**事实投影**作答。
///
/// `warning` 不是事实（宿主局部，见 [D-083 §5.2](../../../../docs/003-design/001-核心架构.md "D-083")），
/// 由调用方原样带上。
fn runtime_from_facts(facts: Option<&RuntimeFacts>, warning: Option<String>) -> SessionRuntime {
    match facts.and_then(|f| f.outcome.clone()) {
        Some(outcome) => {
            let (outcome, error) = outcome.into_state();
            SessionRuntime::finished(&outcome, error, warning)
        }
        None => SessionRuntime::idle(warning),
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
    let warning = inner.last_warning.clone();
    match &inner.last_outcome {
        Some(outcome) => SessionRuntime::finished(outcome, inner.last_error.clone(), warning),
        None => runtime_from_facts(facts, warning),
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

    /// 取**已有**的活跃状态；没有就是没有（不新建）。
    ///
    /// ## 为什么需要它
    ///
    /// `get_or_create` 的"没有就建"对**控制类**动作是错的：中止一个本实例从未
    /// 跑过的会话（典型是子智能体空间的会话被错投到根实例）会顺手造出一个
    /// 永不释放的幽灵状态，而中止本身是空操作——调用方拿到"成功"，什么都没停。
    /// 「这里有没有在跑的东西」必须是**可查询**的，而不是一问就自动变成有。
    pub async fn get(&self, session_id: &str) -> Option<Arc<ActiveSessionState>> {
        self.sessions.read().await.get(session_id).cloned()
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
            return runtime_from_facts(facts, None);
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
    // 一切消息级变更（压缩节点 / 前端 CRUD 动作 / 用户消息定稿 / 失败与中止的
    // 终态收敛）都经 `Transcript::apply` 发布——内存图、seq、核心日志、发布在
    // 同一个函数里完成。VDFS 转写列表的在途叠加读的是同一份内存图，
    // 「存储一份 + 在途一份」两种表示从此同源。
    //
    // 这三个函数住在**名册**上而不是插件上，是因为「按会话 id 解析到它的那份转写」
    // 只需要这张 `sessions` 表——插件在这里没有别的贡献。于是需要发布能力的
    // 一方（压缩发射器）拿的是名册，不是整个插件。

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

    /// **整表重写**（L2 语义压缩）后的收敛：被压掉的消息逐条删除帧，
    /// 新的首条快照作为一条**完整消息**下发（`content` = 整条替换）。
    ///
    /// 逐条删除帧是元数据（id + 状态，每条几十字节），条数受上下文窗口上界
    /// 约束——比「清空 + 整份重读」便宜得多：保留的消息不再重传。协议里也没有
    /// 「清空重读」这种形态（消费端要重读只有一条路：自己发现序号缺口）。
    pub(crate) async fn emit_transcript_rewritten(
        &self,
        session_id: &str,
        dropped: &[String],
        head: &cm::ChatNode,
    ) {
        let mut frames: Vec<cm::ChatNode> = dropped
            .iter()
            .map(|mid| crate::symbio_core::llm_removed_frame(mid))
            .collect();
        // 新的首条（压缩快照）：一条完整消息（身份 + 正文 + 终态同帧）。
        frames.push(crate::symbio_core::llm_message_frame(head));
        self.transcript_apply_all(session_id, frames).await;
    }
}

#[cfg(test)]
#[path = "active.test.rs"]
mod tests;
