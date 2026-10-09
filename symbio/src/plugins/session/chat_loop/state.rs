//! 会话主循环的**状态与契约**。
//!
//! 含会话上下文、请求快照、
//! 单轮状态、闸门判定结果、退出原因，以及编排器与 Stop 信号。
//!
//! 可见性：`ChatOrchestrator` / `StopSignal` 是**跨模块契约**（`orchestrator.rs`、
//! `resume.rs` 经 `chat_loop::X` 引用）故为 `pub`；其余为 `pub(super)`——
//! 只在 `chat_loop` 及其子模块内可见。

use super::super::plugin::PublishTarget;
use super::progress::ProgressPolicy;
use super::*;

/// 本请求所属会话的主体身份——**唯一**读 `ctx[AGENT_ID]` 并派生主体名的取值点。
///
/// 派生本身住 [`crate::symbio_core::authz::principal_of`]（部署事实的唯一 owner）；这里只负责
/// 「从请求上下文取那个 id」。写事件的 `actor`、写消息的 `principal`、写侧闸判的
/// 对象三者都走它——三处各推一次迟早漂移成「判的是 A、写的是 B」。
pub(crate) fn request_principal(ctx: &dyn crate::symbio_core::PluginInvokeRequest) -> String {
    use crate::symbio_core::PluginInvokeRequestExt;
    crate::symbio_core::authz::principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref())
}

/// MODEL 会话上下文
///
/// 设计说明：
/// - 仅包含消息列表，不包含 MODEL 请求配置
/// - system_prompt、tools、轮次上限等配置应从 model_chat::Request 获取
/// - session 用于管理会话历史（滑动窗口/自动截断/持久化）
pub(crate) struct SessionContext {
    pub messages: Vec<ChatMessage>,
    pub session: Arc<PersistentChatSession>,
    /// **本会话的主体身份**（[plan/11 批 1](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
    /// ② 的取值点）：会话选定的 agent ⇒ 主体名，未选 ⇒ `agent:main`。
    ///
    /// 派生只有一处（[`crate::symbio_core::authz::principal_of`]），转写事件的 `actor`、消息的
    /// `principal`、写侧闸判的对象三者共用它——三处各写一份字符串迟早漂移成
    /// 「判的是 A、写的是 B」。放在上下文里而不是每个阶段各推一次：一个事实一个
    /// 取值点，且它随会话走、不随阶段变。
    pub principal: String,
}

/// 请求级不可变配置（`model_chat::Request` 的取值快照，全程只读）。
///
/// 收口前，`req.xxx.unwrap_or(cfg_defaults.yyy)` 散落在主循环各处；现在只在构造
/// 这一份快照时求值一次，下游阶段函数只读快照字段。默认值的唯一真源仍是
/// `SessionConfig`（审计 C2）——本结构只是它的**请求级投影**，不引入第二份默认值。
///
/// 快照**只收主循环真正要读的字段**：`provider_id` 不在这里——provider 的选择发生在
/// **注册期**（model 插件 traverse 时按 `PROVIDER_ID` 解析出唯一生效 provider），
/// 循环内再持一份只会变成永不读的死字段（错误文案要用的那一份在
/// `orchestrator::consume::run_chat_loop_task` 的参数里）。
pub(crate) struct TurnRequest {
    /// 显式软上限（`None` = 不限制；`Some(0)` 与 `None` 同义）
    pub(crate) max_tool_rounds: Option<usize>,
    pub(crate) auto_compress: bool,
    pub(crate) enable_compact_tool: bool,
    pub(crate) tool_context_window: usize,
    pub(crate) load_history: bool,
    /// 请求显式给的系统提示词（`None` = 只有注册段；不吞掉注册段，见
    /// `chat_loop::inputs::resolve_system_prompt`）
    pub(crate) system_prompt: Option<String>,
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
        }
    }
}

/// 轮次状态：请求作用域内的可变状态。
///
/// 收口前这些量散落在 `run_chat_loop` 的循环作用域里，并以 6 个 `&mut` 参数逐个
/// 穿进 `close_turn`；现在收成一个结构体，主循环与阶段函数共享同一份状态。
#[derive(Default)]
pub(crate) struct TurnState {
    /// 用户中止信号（`provider.execute_turn` 与工具执行共享同一份）。
    ///
    /// 收口前是一个裸 `Arc<AtomicBool>`，外部置位要靠「往执行期通道投 Abort 帧」；
    /// 现在是 [`ExecAbortSignal`]——置位与唤醒是同一个动作，不再需要帧与轮询。
    pub(crate) abort: ExecAbortSignal,
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
    /// 本轮的**代际立约记录**（[04 §3.1 批⑧](../../../../docs/plan/04-工程落地.md)，S08 §3）。
    ///
    /// 由工具执行层填（`process_tool_calls_async` 的出参），随收束转写入格
    /// （`v2_facts::record`）——两者之间必须有个**请求作用域**的地方存它：执行在
    /// `close_turn`、入格在 `finish_turn`，中间隔着一整段本轮收尾。与 `TurnState`
    /// 的其它字段一样随请求复位，所以不会把上一轮的承诺带到这一轮。
    pub(crate) delegations: Vec<crate::plugins::session::tools::Delegation>,
    /// 本轮的**任务表声明**（[04 §3.1 批⑨](../../../../docs/plan/04-工程落地.md)，S7 步 16）。
    ///
    /// 形态与 [`TurnState::delegations`] 完全对称：工具执行层填
    /// （`process_tool_calls_async` 的出参），轮末收束转写入格（`v2_facts::record`
    /// → `v2_tasks::write`），中间隔着同一段本轮收尾，所以要有个请求作用域的量存它。
    pub(crate) task_decls: Vec<crate::plugins::session::tools::TaskDeclaration>,
    /// 本轮**外部执行闸门判熔断**的理由（[04 §3.1 批⑩](../../../../docs/plan/04-工程落地.md)，
    /// S8 步 20，[roadmap/S09 §6](../../../../docs/plan/roadmap/S09-外部执行与熔断.md) 验收 2）。
    ///
    /// 形态与 [`TurnState::task_decls`] 对称：工具执行层填（`process_tool_calls_async`
    /// 的出参），轮末收束转写入格（`v2_facts::record` → `CircuitBreaker::break_event`）。
    /// 只有 `Break` 进这张表——`Refuse`（未授权）按验收 1 **不得产生事件**，
    /// 两种拒绝在事件面上必须分得开；这也是为什么它不复用 `delegations` 或
    /// `task_decls` 的出参：那是「做了什么」，这是「**不允许做**什么」。
    pub(crate) gate_breaks: Vec<&'static str>,
    /// **对话线上最近一次动静**的时刻（毫秒）——中途汇报的静默时钟起点。
    ///
    /// 两个来源都算一次"动静"：用户发言（轮首输入 / 轮边界折进的补充）与助手写下
    /// 一句面向用户的话（首响 / 答话 / 上一次汇报）。汇报判定的全部内容是
    /// "距上次动静够久了吗"（见 `progress.rs`）。
    ///
    /// 它是**请求作用域**的量而不是会话级的：`run_chat_loop` 构造 [`TurnState`] 时
    /// 取一次当前时刻，此后每次说话更新——判定点与更新点都在同一个任务里，
    /// 不需要跨任务共享，也就不会有"两个写入者各写一半"的形态。
    pub(crate) last_user_facing_at: i64,
    /// 本轮已汇报次数（中途汇报的配额，见 `progress.rs`）。
    ///
    /// 它随请求复位（`TurnState` 即请求作用域）：`progress_max_per_turn` 说的是
    /// "这一轮最多打断几次"，跨轮累加会让第二次请求一开始就没有配额。
    pub(crate) progress_reports: u32,
    /// 本轮模型调用的**实测累计耗时**（毫秒，[ADR-044](../../../../docs/decisions/core.md)：
    /// 实测与判据同源）。在「LLM 调用唯一发起处」用 `Instant` 累计——含工具轮
    /// 的多次请求；随轮次收束经 v2 事实桥写进事件网格的 `cost_ms`。
    pub(crate) model_elapsed_ms: u64,
    /// 本轮**输入**（消息 id + 正文），在 `single_message` 被消费之前锚定。
    ///
    /// 锚定而不现取，是因为 `context.messages` 装着**整段历史**（`load_history = true`
    /// 时每轮都重新加载）：从里面找「本轮用户发言」找到的永远是首轮那句——转写
    /// （`v2_facts::record` 的 `user.message` 文本与 `attempt` 判据）和记忆编码
    /// （`v2_memory::encode`）会**一起**逐轮记错同一条事实。两处消费同一份锚，
    /// 判决（`first_utterance`）与转写才不会各读各的。
    ///
    /// `None` = 本请求没有用户新发言（`resume` 重跑等）——那时调用方回落到
    /// `v2_facts::first_user_utterance` 的兜底口径（历史首条）。
    pub(crate) input_utterance: Option<(String, String)>,
    /// 本轮的**长期记忆召回视图**（S5 步 12，`v2_memory::recall_view`）。
    ///
    /// 只在本轮**第一个工具轮**取一次（与 classify / history 同一条 `tool_rounds == 0`
    /// 口径）：同一轮内记忆不该漂移，事实源也只扫一遍；后续工具轮复用同一份视图，
    /// 渲染 [`chat_loop::inputs`] 每次调用现算（纯内存，零 I/O）。
    ///
    /// 落 `memory.recalled` 的时机在**轮末收束**（`v2_facts::record`）而不是取视图
    /// 那一刻：溯源锚是本轮 `user.message` 格，那时它才在事实源里。
    pub(crate) recall_view: Option<crate::symbio_core::RecallView>,
    /// 本轮**技能路由**的判定（S9 步 22，[04 §3.1 批⑪](../../../../docs/plan/04-工程落地.md)）：
    /// 每条 `(skill_id, fallback)`，即 `v2_skills::route` 对召回视图里一条技能的判决。
    ///
    /// 形态与 [`TurnState::gate_breaks`] / [`TurnState::task_decls`] 完全对称：
    /// **读侧填**（`prepare_turn_inputs` 拿置信度闸判），**轮末收束入格**
    /// （`v2_facts::record` → 一条 `memory.recalled` 载荷 `{skill_id, fallback}`），
    /// 中间隔着同一段本轮收尾，所以要有个请求作用域的量存它。
    ///
    /// 为什么不能在收束那一刻现算：回退的判决同时**改写了 `recall_view`
    ///**（低置信的技能被摘出本轮视图），事后重算看到的是已经被改过的视图，
    /// 判决与它作用的对象对不上；而且溯源锚要等到 `user.message` 落盘才存在。
    /// 空表 = 本轮没召回技能 / 没判（与 `None` 的「取都没取」不同，但收束侧
    /// 一视同仁）。
    pub(crate) skill_route: Vec<(String, bool)>,
    /// 本轮**可用**的技能集（S11 快路的候选，`v2_skills::route` 的出参之一）：
    /// 过了置信度闸、且本轮发言可能命中的那些。
    ///
    /// 与 [`TurnState::skill_route`] **同一次判定**的两个出口（同一个 `match`）：
    /// 观测供校准归并，这一份供执行侧装配反射档。分两处各判一遍必然漂移成
    /// 「摘出视图的那条」与「拿去执行的那条」不是同一条。
    ///
    /// 它只在 `skill_fast_path` **开着且档位是 `full`** 时被填充（见
    /// `chat_loop::inputs` 的 `fast_armed`）：跳过的是 v2 运行器里的那一次模型调用，
    /// `bridge` / `off` 档的轮次由 v1 执行，没有可跳过的东西。开关关着时这里是空表，
    /// 但**判定照常发生**（`route` 只在 `off` 档不算）——观测照常落格，那正是
    /// 「只编译、不加速」的观察形态；只是没人拿这一份去装配。
    pub(crate) skill_hits: Vec<crate::plugins::session::v2_skills::SkillLlmHit>,
    /// 本轮的**调度段**（S7 步 16–17，[04 §3.1 批⑨](../../../../docs/plan/04-工程落地.md)）：
    /// 就绪任务集渲染成的一段提示，交给**本轮的执行者**（模型）。
    ///
    /// 与 [`TurnState::recall_view`] 同一条取用口径（`tool_rounds == 0` 取一次、
    /// 本轮各工具轮复用）：同一轮内任务集不该漂移，也省得每个工具轮都重开一次
    /// 事实源。它**只读不写**——算不出来（没开过任务 / 就绪集空 / `off` 档）就是
    /// `None`，请求视图里就少一段，不存在"空占位"。
    pub(crate) ready_section: Option<String>,
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
    /// （否则消费循环的 `persist_failure` 会把已成功的 Turn 误标为 `Aborted`）。
    AbortedAtBoundary,
    /// 显式软上限：先广播明确提示再退出，绝不静默。
    MaxToolRounds { max: usize },
    /// 用户中止且**在途 Turn 尚未定稿**：冒泡 `Err(Aborted)`，由消费循环落库为
    /// `MessageStatus::Aborted` + 会话结局 `aborted`（**不是** `failed`），
    /// 前端渲染错误条与重试入口。
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
    pub(crate) tools_done: Vec<TurnToolCallInfo>,
    pub(crate) finish: ModelFinishReason,
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
    ctx: Arc<dyn PluginInvokeRequest>,
    fired: AtomicBool,
}

impl StopSignal {
    pub fn new(parent: Option<Arc<dyn Plugin>>, ctx: Arc<dyn PluginInvokeRequest>) -> Self {
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
/// 把「正在压缩」作为**消息节点**呈现的发射器（压缩期 UI 的唯一出口）。
///
/// ## 为什么是消息节点，而不是会话级提示
///
/// 压缩是会话里真实发生的一步。用户应当像看到一次工具调用那样看到它：
/// 有自己的位置（当前时刻）、自己的状态（进行中 / 已完成），被压掉的历史
/// 就发生在它之前。挂在窗口顶部的横幅没有位置概念——用户滚到消息流中间时
/// 看不见它，事后也无法回溯「上次压缩发生在哪里、压掉了多少」。
///
/// ## 为什么能在静音窗口里发出去
///
/// 压缩 LLM 请求的出帧被刻意静音（`send_compression_request` 用哑 `tx` 接住
/// 全部流式帧，以免泄漏一个永不 finalize 的空 Turn 骨架）。但本发射器走
/// `emit_message_patch` → VDFS 变更订阅，**不经过**那条被静音的 turn channel。
///
/// ## 为什么持有插件而不是「一个回调」
///
/// 发变更需要两样东西：store（取会话摘要）与 `change_subs`（投递），两者都在插件上。
pub struct CompressionEmitter {
    plugin: Arc<crate::plugins::session::plugin::SessionPlugin>,
    pub(crate) state: Arc<crate::plugins::session::active::ActiveSessionState>,
}

impl CompressionEmitter {
    pub fn new(
        plugin: Arc<crate::plugins::session::plugin::SessionPlugin>,
        state: Arc<crate::plugins::session::active::ActiveSessionState>,
    ) -> Self {
        Self { plugin, state }
    }

    /// 插入「正在压缩」节点：进在途图 + 发布（经 Transcript 唯一入口）。
    ///
    /// 进在途图是必要的：会话叶子 `read` 会叠加在途（见 `overlay_live`），
    /// 因此压缩期间切走再切回，这个节点仍然可见——否则用户切回来只看到「什么
    /// 都没有」，又变回最初那个"卡死"观感。
    pub async fn begin(&self, node_id: &str) {
        // 一条完整消息：身份 + 状态 + 占位正文一帧到位。
        self.state.transcript.lock().await.apply(ChatMessage {
            id: node_id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Compression),
            status: Some(MessageStatus::Streaming),
            content: Some(MessageContent::Text("正在压缩上下文…".to_string())),
            ..Default::default()
        });
    }

    /// 定稿节点并使其离开在途图，返回终态副本供调用方落库。
    ///
    /// 在途图里找不到该 id 时（例如前端是在压缩开始之后才连上的）**照样构造
    /// 并发布**——终态必须到达，否则那个 `Streaming` 节点会永远留在前端转圈。
    ///
    /// `failure_kind`：失败原因码（写进 `meta.failure_kind`）。
    ///
    /// `stats`：这次压缩的**结构化交代**（来源 / 前后水位 / 上限 / 丢了几条）。
    /// 正文只说得了一条「N → M 条」，而用户真正想知道的是「我离上限还有多远、
    /// 这次是谁触发的」——那些都是字段，不是文案，因此随 `meta` 下发由前端渲染。
    /// 缺字段时前端退回正文那一行，旧前端（不认识这些字段）也不受影响。
    pub async fn finish(
        &self,
        node_id: &str,
        status: MessageStatus,
        text: &str,
        failure_kind: Option<&str>,
        stats: Option<serde_json::Value>,
    ) -> ChatMessage {
        let node = {
            let mut tr = self.state.transcript.lock().await;
            let mut node = tr.get(node_id).unwrap_or_else(|| ChatMessage {
                id: node_id.to_string(),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Compression),
                ..Default::default()
            });
            node.status = Some(status.clone());
            node.content = Some(MessageContent::Text(text.to_string()));
            if let Some(kind) = failure_kind {
                merge_meta(&mut node, serde_json::json!({ "failure_kind": kind }));
            }
            if let Some(stats) = stats {
                merge_meta(&mut node, stats);
            }
            // 发布终态 + 落库回执：权威副本即将由调用方落库，
            // 在途副本必须作废（否则同一条消息以「存储 + 在途」两种形态参与叠加）。
            //
            // **完整消息帧**（不是状态帧）：本节点的正文从未经 `delta` 上线过——
            // `begin` 发的是一句占位（"正在压缩上下文…"），这里的正文（"已压缩
            // N → M 条"）是**首次也是唯一**一次上线。用状态帧剥掉正文，前端会一直
            // 停在占位文案上，直到重开会话才从存储读到结果（实测回归）。
            // 状态帧只适用于「正文已由 delta 逐帧上线」的节点。
            tr.apply(crate::symbio_core::llm_message_frame(&node));
            tr.persisted(std::slice::from_ref(&node.id));
            node
        };
        node
    }

    /// 整表重写（L2 语义压缩）后的收敛发布。
    ///
    /// 与上面两个方法的区别：它们发布的是"压缩这一动作本身"的节点，这条发的是
    /// **转写列表被重写**这件事——被压掉的消息逐条 Remove，新的首条（快照）
    /// Upsert。没有它，前端会一直显示压缩前的历史。
    pub async fn emit_rewrite(&self, session_id: &str, dropped: &[String], head: &ChatMessage) {
        self.plugin
            .emit_transcript_rewritten(session_id, dropped, head)
            .await;
    }

    /// 压缩请求的**增量改道落点**：返回直连本会话转写唯一写入点的 writer（仅由
    /// `context::pipeline` 的过滤桥消费——摘要帧经白名单改写后从这里进转写）。
    ///
    /// ## 为什么复用转写而不是另开一条通道
    ///
    /// 增量落点是「在途图 + 位置号 + 发布」三件事的合成：另开通道等于把这套收敛
    /// 逻辑再实现一遍，且必然漂移。压缩期本转写的唯一常规写入者（消费循环）正等
    /// 着压缩完成，不存在写竞争——这与 `begin` / `finish` 经同一入口写是同一个理由。
    pub(crate) fn transcript_writer(
        &self,
    ) -> std::sync::Arc<dyn crate::symbio_core::ExecTranscriptWriter> {
        std::sync::Arc::new(TranscriptWriterBridge {
            transcript: self.state.transcript.clone(),
        })
    }

    /// 落库回包用的**发布通道**（[`PublishTarget::Transcript`]：按会话 id 现场解析转写）。
    ///
    /// 「落库 → 逐条下发权威副本」整段动作在 `plugin::append_and_publish`（§3.4 的
    /// 唯一落地点）里，这里只负责给出通道——压缩域没有自己的 `ExecEventSink`，
    /// 压缩期的出帧是被静音的（见模块头）。
    ///
    /// 载荷是 `append_messages` 交回的**权威副本**（`content` 整条替换），不是 `finish`
    /// 返回的那份——后者没有号（补号发生在存储临界区内的私有副本上）。`finish` 发的
    /// 那一帧带的是转写分配的**在途号**（`1 << 50`），号只由存储在写入时分配，
    /// 落库后必须再发一次权威副本才能换回。不回包的后果是静默的：该节点永远排在
    /// 全部存储号之后，下一条用户消息（小存储号）会跳到它**前面**，前端于是看到
    /// 压缩节点跑到对话末尾去。
    pub(crate) fn target<'a>(&'a self, session_id: &'a str) -> PublishTarget<'a> {
        PublishTarget::Transcript {
            plugin: &self.plugin,
            session_id,
        }
    }
}

/// 补充整合在**轮边界**的落点：抽干整队并合并成一条用户消息。
///
/// ## 为什么需要它（与 [`CompressionEmitter`] 同一个理由）
///
/// `run_chat_loop` 是自由函数，拿不到插件实例；而"抽干"要读两样都在插件上的东西：
/// `active_mgr`（会话队列）与 `config`（`supplements_enabled` / `supplements_max_per_drain`）。
/// 所以由调用方（`orchestrator::consume`）在构造 `ChatOrchestrator` 时把这个持有者
/// 一并交进来——与 `compression` 字段同形，不新增第二种注入手法。
///
/// ## 它只做一件事
///
/// 取批（[`SessionPlugin::take_inbox_batch`]）+ 合并（`merge_supplements`），
/// **不写任何地址、不落库**：合并消息由主循环推进 `context.messages`，
/// 落库仍走既有的锚点增量路径（见 `run_chat_loop` 的轮边界抽干点）。
pub struct SupplementDrain {
    plugin: Arc<crate::plugins::session::plugin::SessionPlugin>,
    pub(crate) state: Arc<crate::plugins::session::active::ActiveSessionState>,
}

/// 抽干结果：**正文 + 原条数**。
///
/// `InboxItem` 是 `transcript::inbox` 的私有类型，所以同步孪生不返回它——
/// 跨模块暴露内部类型会让「谁都能构造一个假的抽干结果」成为可能。
///
/// ⚠️ **不带原始 `ChatMessage`**：v1 路径要落进 `context.messages`，但它走的是
/// 另一个方法（`drain`，返回 `ChatMessage` 本身）。这里只服务 v2 注入，而 v2 只要
/// 正文——多带一个字段就是给下一个人留一个不必有的理由。
#[derive(Debug, Clone)]
pub(crate) struct DrainedSupplement {
    /// 折进 prompt 的正文（多条已合并）。
    pub text: String,
    /// **原条数**：合并了 n 条时是 n。
    ///
    /// 记下来是因为「模型收到一大段」与「用户连说了 5 句」对模型是不同的输入，
    /// 而事实网格不该只保留前者。
    pub count: u64,
    /// 合并消息的 **id** = **第一条补充的 id**（`merge_supplements` 的既有约定）。
    ///
    /// 为什么必须沿用而不是新造：前端按 id 合并权威帧（`useChatConnection.ts`），
    /// 换 id 等于让同一句话在前端**出现两条**。t19 的 B 幕断言钉的就是「沿用 b2」。
    pub id: String,
    /// 批内**原始条目 id 列表**（顺序 = 入队顺序）。
    ///
    /// 落进 `meta.supplement_ids`：前端与事后审计都要能回答「这一条合并了几条、
    /// 分别是哪几条」（t19 的 B 幕断言钉的就是这个列表）。
    pub ids: Vec<String>,
}

impl SupplementDrain {
    pub fn new(
        plugin: Arc<crate::plugins::session::plugin::SessionPlugin>,
        state: Arc<crate::plugins::session::active::ActiveSessionState>,
    ) -> Self {
        Self { plugin, state }
    }

    /// 抽干该会话队首的一批补充并合并成**一条**用户消息。
    ///
    /// 返回 `None` = 队列为空（或合并结果为空批次）——调用方据此**什么都不做**，
    /// 不推进 `last_saved`、不落库。
    ///
    /// 开关与上界的判定在 [`SessionPlugin::take_inbox_batch`] 内（那里是唯一真源）：
    /// `supplements_enabled = false` 时它退化为"取一条"，与本函数的合并叠加后
    /// 恰好等于"一条消息 = 一轮"的今天行为。
    pub async fn drain(&self) -> Option<ChatMessage> {
        let batch = self.plugin.take_inbox_batch(&self.state).await;
        crate::plugins::session::transcript::supplements::merge_supplements(&batch)
    }

    /// [`Self::drain`] 的**同步**孪生（缺口 3：v2 工具循环里的注入挂点）。
    ///
    /// ## 为什么需要它
    ///
    /// core 的 [`TurnInjector`] 是同步闭包（`Fn() -> Vec<PromptMessage>`）——core 里
    /// 不该为了「等一个异步取件」而开出 async trait，那会把整个 `run_with_tools`
    /// 的接口面染成 async（它现在是同步调 LLM 适配器的）。
    ///
    /// 于是取件这一步在插件侧桥成同步。**判定与取件逻辑不复制**：与
    /// [`Self::drain`] 共用同一个真源（`take_inbox_batch` 的同步版），差别只在
    /// 锁的等法。
    ///
    /// ⚠️ **调用方负责 `block_in_place`**：本函数内部用 `blocking_read`，若在
    /// 单线程 runtime 的 async 上下文里直接调会 panic。`v2_exec` 的注入闭包里有
    /// flavor 判定（多线程才 `block_in_place`，否则裸调）。
    pub fn take_inbox_batch_sync(&self) -> Option<DrainedSupplement> {
        let batch = self.plugin.take_inbox_batch_sync(&self.state);
        let merged = crate::plugins::session::transcript::supplements::merge_supplements(&batch)?;
        let count = merged
            .meta
            .as_ref()
            .and_then(|m| {
                m.get(crate::plugins::session::transcript::supplements::META_SUPPLEMENT_COUNT)
            })
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        Some(DrainedSupplement {
            text: merged
                .content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default(),
            count,
            id: merged.id,
            ids: merged
                .meta
                .as_ref()
                .and_then(|m| {
                    m.get(crate::plugins::session::transcript::supplements::META_SUPPLEMENT_IDS)
                })
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// [`CompressionEmitter::transcript_writer`] 的落点：一个只做「锁转写 → apply」
/// 的最小 writer（合成 [`crate::symbio_core::ExecTranscriptWriter`] 的桥，与
/// `orchestrator::sink::TranscriptSink` 平行——后者多了会话级告警分派，压缩增量
/// 用不上）。
struct TranscriptWriterBridge {
    transcript: std::sync::Arc<tokio::sync::Mutex<super::super::transcript::Transcript>>,
}

#[async_trait::async_trait]
impl crate::symbio_core::ExecTranscriptWriter for TranscriptWriterBridge {
    async fn apply(&self, message: ChatMessage) {
        self.transcript.lock().await.apply(message);
    }
}

/// 把一个字段补丁（对象）**合并**进节点 `meta`，不覆盖已有键——
/// `failure_kind` 与统计字段因此可以各写各的，谁都不会把对方抹掉。
fn merge_meta(node: &mut ChatMessage, patch: serde_json::Value) {
    let serde_json::Value::Object(patch) = patch else {
        return;
    };
    let mut meta = node
        .meta
        .take()
        .filter(|m| m.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = meta.as_object_mut() {
        for (k, v) in patch {
            obj.insert(k, v);
        }
    }
    node.meta = Some(meta);
}

/// ## 为什么没有 `new()`
///
/// 字段全是 `pub`，构造点是**唯一**的（`orchestrator/consume.rs`）。此前有一个
/// 八参的 `new()`——那是八个**位置参数**，读调用点要靠数数才知道谁是谁，而且
/// 每加一个字段就要再挤一个（`clippy::too_many_arguments` 在第八个就报错）。
/// 结构体字面量把"谁是谁"写在字段名上，**字段增删由编译器在唯一构造点报错**，
/// 比位置参数强。这里不再补 `new()`。
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
    /// 压缩节点的发射器（把「正在压缩」作为消息节点呈现）。
    ///
    /// `None` = 调用方没有提供（测试 / 无前端场景）：压缩照常执行，只是不呈现节点。
    /// 之所以是可选而非必填：`run_chat_loop` 的其它调用场景（单测）根本没有
    /// 会话状态与前端订阅者，让它们为「一个提示」去构造插件实例是本末倒置。
    pub compression: Option<Arc<CompressionEmitter>>,
    /// **轮边界补充整合**的落点（把运行中到达的补充抽干、合并成一条用户消息）。
    ///
    /// `None` = 调用方没有提供（单测 / 无会话状态场景）：轮边界不抽干，
    /// 行为与改造前一致。可选而非必填的理由与 `compression` 相同。
    pub supplements: Option<Arc<SupplementDrain>>,
    /// 轮首判决的**请求级开关快照**（`SessionConfig::classify_enabled`）。
    ///
    /// ## 为什么是一个值而不是一个持有者
    ///
    /// `compression` / `supplements` 必须持有插件，因为它们要读会话状态与配置的
    /// **当前值**（压缩水位、队列深度）；判决只需要一个布尔：配置在一次请求的生命
    /// 周期内不变，因此在这里取一次快照就够——多持一个 `Arc<SessionPlugin>` 只会
    /// 多一条可以绕过配置面的路径。
    ///
    /// ## 它关掉的是什么
    ///
    /// `false` ⇒ 本循环**不调用** `classify/decide`，全部输入直接进工具循环——
    /// 与未挂载该插件时的行为一致（两条路径都退化成"今天的行为"，但验证的
    /// 是两件不同的事：这里是"分支写对了"，卸载是"插件边界真的存在"）。
    pub classify_enabled: bool,
    /// 对话面措辞的**请求级开关快照**（`SessionConfig::compose_enabled`）。
    ///
    /// ## 为什么也是一个值
    ///
    /// 与 [`ChatOrchestrator::classify_enabled`] 同一条理由：配置在一次请求生命周期内
    /// 不变，措辞只需要一个布尔。持有 `Arc<SessionPlugin>` 会多一条绕过配置面的路径，
    /// 而这里没有任何"当前值"要读。
    ///
    /// ## 它关掉的是什么
    ///
    /// `false` ⇒ 本循环**不调用** `compose/compose`：`Answered` 拿不到措辞，
    /// 于是**降级进工具循环**（不沉默），`Escalate` 没有首响。与未挂载 `compose`
    /// 时的行为一致——两条路径都退化成"没有对话面文本"，但验证的是两件不同的事
    /// （这里是"分支写对了"，卸载是"插件边界真的存在"）。
    ///
    /// ## 它与 `classify_enabled` 是**两级独立的开关**
    ///
    /// 判决决定"要不要干活"，措辞决定"说什么"。四个组合都成立且都有意义：
    /// 只判决不措辞（判决决定派活与否，话由工具循环说）、只措辞不判决（没有判决
    /// 就没有 `Answered`，措辞只剩 `Escalate` 首响——本批 `Escalate` 由 `classify`
    /// 产出，故该组合退化为"不生效"，但**结构上合法**，不是需要拦的错误）。
    pub compose_enabled: bool,
    /// 中途汇报的策略快照（`SessionConfig` 的四个旋钮，见 [`ProgressPolicy`]）。
    ///
    /// 与 `classify_enabled` / `compose_enabled` 同形（取**值快照**），但这里是一个结构体
    /// 而不是四个平铺字段：它们是**同一个判定**的四个参数（见
    /// [`ProgressPolicy::due`]），拆成四个字段会让"谁和谁是一组"只能靠命名猜。
    ///
    /// 四个参数各自能取到的值见 `SessionConfig`；构造点唯一
    /// （`orchestrator/consume.rs`）。
    pub progress: ProgressPolicy,
    /// **本插件自己的目录**（装配期由父插件经 `PLUGIN_DIR` 告知）。
    ///
    /// 会话存储 / 转写存档 / 工具结果存档都在这个目录下——它是「本实例的作用域」，
    /// 顶层时恰好是系统根，挂在子智能体下时就不是。执行期**不再**从请求上下文
    /// 反推：请求上下文不带 `PLUGIN_DIR`（那是装配期键），反推必然落到父作用域。
    pub session_dir: crate::symbio_core::PluginDir,
}

impl ChatOrchestrator {
    pub async fn finalize_assistant_turn(
        &self,
        root_id: &str,
        out: &TurnOutput,
        sink: &ExecEventSink,
    ) {
        if out.is_reasoning_only() {
            // reasoning-only：模型只产生了 reasoning，没有独立的文本回复。
            //
            // 同一段 reasoning 在落库时由 llm_build_assistant_messages 以「Text 响应子节点」承载
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
                // 该节点已由 ReasoningDelta 逐帧上线过正文，这里只迁状态。
                llm_emit_state(
                    sink,
                    ChatMessage {
                        id: out.reasoning_child_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Reasoning),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(MessageStatus::Completed),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                let resp_id = if out.response_text_child_id.is_empty() {
                    llm_short_id()
                } else {
                    out.response_text_child_id.clone()
                };
                // 兜底路径：这个 Text 节点是**新建**的（reasoning-only 时没有正文
                // 子节点），正文必须随帧上线 ⇒ 完整消息帧。
                llm_emit_message(
                    sink,
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
            // 根 Turn 的终态**不在此发出**：Turn 是组合节点（仅分组，无正文），
            // 终态必须晚于子树——而本轮的 ToolCall 要到 `close_turn` 里才执行完。
            // 唯一发射点是 `run_chat_loop` 的 `finalize_turn_root`（本轮收尾点）。
            return;
        }

        // Mark reasoning child as completed
        if !out.reasoning.is_empty() && !out.reasoning_child_id.is_empty() {
            llm_emit_state(
                sink,
                ChatMessage {
                    id: out.reasoning_child_id.clone(),
                    parent_id: Some(root_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Reasoning),
                    content: Some(MessageContent::Text(out.reasoning.clone())),
                    status: Some(MessageStatus::Completed),
                    ..Default::default()
                },
            )
            .await;
        }

        // Mark response text child as completed (exists if there was text content)
        if !out.text.is_empty() && !out.response_text_child_id.is_empty() {
            llm_emit_state(
                sink,
                ChatMessage {
                    id: out.response_text_child_id.clone(),
                    parent_id: Some(root_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(out.text.clone())),
                    status: Some(MessageStatus::Completed),
                    ..Default::default()
                },
            )
            .await;
        }

        // ToolCall 子节点**不在这里定格**。
        //
        // 这里只表示"模型停止输出参数"，不表示"这次工具调用结束了"：节点此后还要
        // 走完一整段**执行窗口**（一次编译 / 一次网络请求 / 一个子智能体跑完，
        // 往往比参数流式本身长得多）。若在此标 `Completed`，前端在整段窗口里就
        // 没有任何「运行中」迹象——参数流完画面静止，直到结果突然出现，用户无法
        // 判断是"还在跑"还是"卡死了"。
        //
        // 因此 ToolCall 的终态只由**执行方**给出，且恰好一处：
        // - 正常分发：`tool_executor::process_tool_calls_async`（执行前 `Streaming`，
        //   执行后 `Completed` / `WaitingUserAction`；未执行的批尾统一收口）
        // - 恢复执行：`resume::process_tool_resume_action`（approve/retry/supply）
        //
        // 状态机见 `docs/node-state-streaming.md` §2.3；
        // 「每个 ToolCall 必然到达终态」这条不变量由上面两处负责保证。

        // 根 Turn 的终态**不在此发出**。
        //
        // LLM 流结束只说明「模型这一轮说完了」，不说明「这一轮结束了」：Turn 是
        // **组合节点**（仅分组、无正文），它的终态必须**跟随子树**——而本轮的
        // ToolCall 此时连执行都还没开始（执行在 `close_turn` 内，含整个执行窗口）。
        // 若在此标 `Completed`，树上就会出现"容器已完成、其中的工具调用仍在运行"
        // 这种自相矛盾的形状（实测抓包 seq 5 早于 seq 6–8 即此）。
        //
        // 唯一的发射点是 `run_chat_loop` 在 `close_turn` 返回之后的
        // [`finalize_turn_root`]：那一刻子树才真正收敛，且发出的正是**即将落库的
        // 那条节点**——live 与 storage 按同一取值收敛。
    }
}

#[cfg(test)]
#[path = "state.test.rs"]
mod tests;
