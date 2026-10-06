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
//! 今天是"算了却看不见"——读方接进来属 `invariants` 那一侧的独立一批，
//! 见 `execute_turn` 里派生点旁的注记。
//!
//! ## 收束派生事实：记忆与学习（本档已覆盖）
//!
//! 轮次事实由运行器原生入格，`chat_loop` 因此以 `TurnState::v2_executed` 拦下整段
//! `v2_bridge::record`。但**记忆与学习不是轮次事实**，而是本轮的派生副作用——运行器
//! 一处都不写。本档因此在轮末直接调 [`super::v2_bridge::record_learning`]（与 bridge 档
//! **同一个函数**，差别只在溯源锚：这里是原生写的 `u-{turn}` 格）：步 11 编码 /
//! 步 12 检索锚 / 步 13 巩固 / 步 22 技能观测与编译。
//!
//! ## 本档**尚未**覆盖的收束派生事实（诚实缺口）
//!
//! 同一段 `record_to_wal` 里还有四类派生事实，目前仍只走 bridge 档——`full` 档下
//! 它们**不入格**：
//!
//! - **承诺**（`commitment_events`，S08 §3）与**任务表**（`v2_tasks::write`，S7 步 16–18）
//!   与**熔断**（`CircuitBreaker::break_event`，S8 步 20）：三者的数据来源都在工具执行层
//!   （`Delegation` / `TaskDeclaration` / 熔断理由），而 `full` 档的工具经 `DispatchPort`
//!   分发——那三份出参目前没有回到 `v2_exec` 的通道；
//! - **写侧授权闸**（`authorize_close`，[plan/01 §7](../../../docs/plan/01-核心架构.md)）：
//!   bridge 档在落收束格**之前**判 `reply.first` / `reply.append`，运行器不判。
//!
//! 这两组的补法与记忆/学习同形（把出参带进来 + 复用同一个写方），但**数据来源不同**：
//! 前者要先让分发方把三份出参交回（`DispatchPort` 的取件面），后者要先决定运行器
//! 的落格路径怎么接闸——各自是独立一批，不混进本档的收束收尾。
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

use tokio::sync::mpsc;

use crate::symbio_core::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    llm_short_id, ActorSpec, CapabilityMeta, DeltaSink, DispatchPort, Entity, Event, EventWalStore,
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
    /// 本轮开头召回的长期记忆视图（S5 步 12）——轮末由
    /// [`super::v2_bridge::record_learning`] 落一条 `memory.recalled`。
    ///
    /// 它**不是**轮次事实，因此不随 `v2_executed` 的拦截一起消失：v2 原生记账
    /// 只覆盖轮次事实，记忆与学习的写方两档共用同一个函数（见该函数文档）。
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
        recalled,
        skill_obs,
        skill_hits,
    } = req;
    // 事实源：per-session v2 WAL（与桥同一个文件——两档共用一份网格）。
    let dir = session.session_dir().ok_or_else(|| {
        PluginError::InternalError("full 档需要持久会话（临时会话无事实源）".into())
    })?;
    let wal = dir.join(super::paths::V2_WAL_FILE);
    let store = EventWalStore::open(&wal)
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
    // 属 `invariants` 那一侧的独立一批，不混进本批。
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
                    &store,
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
                actor,
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
                    &store,
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
            drop(dispatcher);
            r
        }
    }
    .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;

    // 本轮的正文节点 id 必须在桥 drop 之前取走（收束帧由 chat_loop 的
    // `finalize_assistant_turn` 发出，用的就是它）。
    let child_id = bridge.current_node().unwrap_or_default();
    // 桥归零 ⇒ 通道关闭 ⇒ 发射端排空后退出。
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：运行器**未落收束格**（网格少一格是诚实缺口，ADR-045 同源），
    // 出口走 Aborted——由 chat_loop/消费循环落库为 `MessageStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`），与 v1 的中止出口同形。
    // 记忆与学习也**不写**：与 bridge 档的中止出口同形（那一档的 `TurnExit::Aborted`
    // 同样不转写）——中止的轮次没有收束，没有可固化的东西。
    if outcome.aborted {
        return Err(PluginError::Aborted);
    }

    // ── 收束派生事实：记忆与学习（步 11–13 + 步 22）────────────────────────
    //
    // **本步的存在理由**：`full` 档的轮次事实由运行器原生入格，chat_loop 因此以
    // `v2_executed` 拦下整段 `v2_bridge::record`。但「记忆三段 + 技能观测与编译」
    // 不是轮次事实，而是本轮的**派生副作用**——运行器一处都不写。不在这里补，
    // full 档的长期记忆（S06）与技能自我改进（S11）就整体失效，而档位名还自称
    // 「整体切换」。写方与 bridge 档**同一个函数**（`record_learning`），
    // 差别只在锚点：这里是 v2 原生写的 `u-{turn}` 格，那里是转写的 `v2u-*` 格。
    //
    // 兜底收束的 `response` 为 `None`：兜底说明这条路没走通，固化它等于把失败写成
    // 套路（与 bridge 档 `V2Closure::Fallback` → `success_text = None` 同一条口径）。
    let response = (!outcome.fell_back).then_some(outcome.text.as_str());
    match store
        .range(Seq::new(0))
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE && e.turn == turn_no)
        .and_then(|e| e.seq.map(|s| s.value()))
    {
        Some(user_seq) => super::v2_bridge::record_learning(
            &wal,
            &store,
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
        ),
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
