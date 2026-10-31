//! 会话主循环的**状态与契约**。
//!
//! 含会话上下文、请求快照、
//! 单轮状态、闸门判定结果、退出原因，以及编排器与 Stop 信号。
//!
//! 可见性：`ChatOrchestrator` / `StopSignal` 是**跨模块契约**（`orchestrator.rs`、
//! `wake.rs` 经 `chat_loop::X` 引用）故为 `pub`；其余为 `pub(super)`——
//! 只在 `chat_loop` 及其子模块内可见。

use super::*;
use symbio_core::chat_node::CHAT_STYLE_TAG_REASONING;
use symbio_core::PluginError;

// 「本请求所属会话的主体身份」这个取值点已上移 core（[D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 1 组）：
// 它是 core 既有键 `AGENT_ID` 的纯派生函数，不属于任何一个切片，会话驱动器切出去后
// 读写两侧隔着插件墙都要用。本文件与 `chat_loop` 经这条短路径拿它。
use symbio_core::request_principal;

/// 本请求的**来源主体**（[D-067]）：入站消息的出处，`commitment.*` 的 `to`。
///
/// 取值点 = `metadata.source_principal`（网关 `build_ctx` 落 ctx；收件箱路径由
/// `run_inbox_turn` 从条目寄存挂回）。**缺省 = 本机用户**——与接线前在
/// `derived_facts` 调用点写死的常量逐字一致，本机路径（直呼 / 心跳）行为不变。
///
/// 注意它与 [`request_principal`] 回答的是两个问题：后者是「以谁的身份跑轮」
/// （会话主体，写事件的 `actor`），这里是「这条消息是谁发的」（跨体承诺的
/// 对方）。两个都从 ctx 取但键不同，混用会把「向谁宣告」错写成「谁在宣告」。
///
/// 决策 D-067 见 [D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "D-067")
pub(crate) fn request_source_principal(ctx: &dyn symbio_core::PluginInvokeRequest) -> String {
    use symbio_core::PluginInvokeRequestExt;
    ctx.get(symbio_core::SOURCE_PRINCIPAL)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| symbio_core::PRINCIPAL_USER.to_string())
}

/// 本请求的**主体规格**（`ActorSpec` 的唯一生产构造点，[D-065] 合并批落地）。
///
/// 五个字段各自的数据源：
///
/// | 字段 | 取法 |
/// |---|---|
/// | `principal` | [`request_principal`]（会话选定的 agent ⇒ 主体名） |
/// | `capabilities` | **授权表行**（`production_matrix` ⇒ `grant_names_of`）——单一真源。它有**真读方**：写侧收束闸 `turn_runner::closure_gate` 读规格自己的这份集合（`actor_can_land_speech` / `actor_can_land_artifact`），不再回头查表 |
/// | `pattern` | [`pattern_for_principal`]（判据住在 core 的 actors 定义域，与授权表同源；主体类型名不散出 core——NDC-001 同纪律） |
/// | `budget_ms` | 主体的**声明档**：由 `pattern` 派生（[`declared_budget`]——模式三值本来就按时延分）。它**不是**本幕的预算——本幕的档位要等轮首四行跑完才折得出来（`D-223` 订正 1），而主体在幕首就建（`D-222` 订正 3）。本幕实际用的档与预算归 `turn.dispatch` 那一格（`chat_loop::land_dispatch` 由档位直接折 `budget_ms`），`I3` 读的永远是那一格 |
/// | `scope` | [`LoopSession::scope`]（子会话 ⇒ `Child(父主体)`、顶层会话 ⇒ `Root`；session 从元数据预计算） |
///
/// 决策 D-065 见 [D-065](../../../../docs/003-design/011-Agent目录规范.md#d-065-对等侧收束commitmentasserted-缺的不是写方而是对等-offerassert②-与-③-同一批收束格按种类判 "D-065")
///
/// ## 为什么不吃档位（`D-223` 订正 2）
///
/// 档位是**本幕**的调度参数，而它要等轮首四行跑完才折得出来（`D-223` 订正 1）；
/// 主体却要在**幕首**就建好（`D-222` 订正 3：`Ctx.session` 不是 `Option`，草稿必须先于
/// 任何动作存在）。两者互相派生就成环——所以主体不吃档位，`budget_ms` 记它的**声明档**。
/// 本幕实际用哪一档由 `turn.dispatch` 那一格说，那才是 `I3` 的口径。
///
/// 这不是把一条事实拆成两条：**主体的上限**与**本幕的档**本来就是两个量，用档位填规格
/// 是让「幕的参数」冒充「主体的属性」。
pub(crate) fn request_actor_spec(
    ctx: &dyn symbio_core::PluginInvokeRequest,
    scope: symbio_core::Scope,
) -> symbio_core::ActorSpec {
    use symbio_core::{pattern_for_principal, ActorSpec, PluginInvokeRequestExt};
    let principal = request_principal(ctx);
    // 绑定即登记（[D-100]）：登记的主 owner 是**会话创建点**
    // （`session::commands::session_upsert` 的 create 分支）；这里只在
    // 「本请求显式改写了身份」（请求级 `agent_id`）或临时会话没有创建点时做
    // **幂等兜底**——同一会话重绑只替换自己那一条，行仍随会话回收。
    //
    // 根与子会话的判据不同——根没有父行，未部署时继承会话面缺省行，已部署时原样
    // 用自己的行；委派子会话取「父行 ∩ 自己的声明行」（委派不放大权威）。登记先于
    // 取数，规格身上的能力集才是「现在准不准」的合流面；未绑定的主体照旧 fail-closed。
    let session_id = ctx.get(symbio_core::SESSION_ID).unwrap_or_default();
    match &scope {
        symbio_core::Scope::Child(parent) => {
            symbio_core::bind_inheriting(&session_id, &principal, parent)
        }
        symbio_core::Scope::Root => symbio_core::bind_root(&session_id, &principal),
    }
    let matrix = symbio_core::production_matrix();
    // 模式判据住在定义域内（[`symbio_core::pattern_for_principal`]）——
    // 与授权表同源、与主体类型名一起不散出 core 的 actors 域（NDC-001）。
    // 声明档由它派生（`D-223` 订正 2）：模式三值本来就按时延分。
    let pattern = pattern_for_principal(&principal);
    ActorSpec {
        principal: principal.clone(),
        pattern,
        capabilities: matrix.grant_names_of(&principal),
        budget_ms: declared_budget(pattern),
        scope,
    }
}

/// 主体的**声明档**：模式 ⇒ 时延档（`D-223` 订正 2）。
///
/// 判据就是 [`symbio_core::Pattern`] 自己的文档——三值本来就是按**时延**分的：
/// 「确定，毫秒级」⇒ [`LatencyTier::Reflex`]；「事件 → 另一种表示，百毫秒」⇒
/// [`LatencyTier::Fast`]；「非确定，秒–分钟」⇒ [`LatencyTier::Deep`]。所以这不是
/// 一张新表，是把模式那一行已经写着的话读出来。
///
/// ## 它为什么不是「本幕的档位」
///
/// 本幕走哪一档要等轮首四行跑完才知道（快路作答 ⇒ `reflex`，否则 `deep`），落成
/// `turn.dispatch` 那一格（`chat_loop::land_dispatch`）。主体在幕首就建，所以它只能
/// 带**自己那一类**的档——两个量，两处，不互相冒充。
fn declared_budget(pattern: symbio_core::Pattern) -> u64 {
    use symbio_core::{LatencyTier, Pattern};
    let tier = match pattern {
        Pattern::Decider => LatencyTier::Reflex,
        Pattern::Translator => LatencyTier::Fast,
        Pattern::Reasoner => LatencyTier::Deep,
    };
    tier.budget_ms()
}

/// MODEL 会话上下文
///
/// 设计说明：
/// - 仅包含消息列表，不包含 MODEL 请求配置
/// - system_prompt、tools 等请求配置从 chat_request::Request 获取
/// - session 用于管理会话历史（滑动窗口/自动截断/持久化）
pub(crate) struct SessionContext {
    pub messages: Vec<ChatNode>,
    pub session: Arc<super::super::turn_session::LoopSession>,
    /// **本轮发生过的压缩**（压缩后的整份消息列表）。
    ///
    /// ## 为什么不在压缩现场直接落格
    ///
    /// `session.consolidated` 是**事实**，而事实只能由持写者令牌的一方落格
    /// （[`symbio_plugin_session::store::EventWalStore`] 的单写者闸门）。压缩跑在
    /// 接缝（`run_seam` 那一行），而它**不持**令牌——令牌在 `turn_exec::drive`
    /// 里随 WAL 打开而取。所以在压缩现场落格会拿到
    /// `NotTheWriter`，而那是**静默**的：压缩照常生效、界面照常变短，只有事实源里
    /// 少了一条事件（于是切源之后压缩在重建面上等于没发生过）。
    ///
    /// 于是压缩把「压成了什么」寄存在这里，由 `turn_exec::drive` 落格并归档
    /// （见 `turn_exec::consolidate_into_wal`）。寄存而不是就地落格，是**唯一**能同时
    /// 满足「事实必须落格」与「落格必须持令牌」两条约束的形态。
    ///
    /// 幂等由落格那一侧保证（与 WAL 里最近一份快照逐 id 比对）：同一轮里
    /// `drive` 可能被调多次，而压缩只发生一次。
    pub consolidated: Option<Vec<ChatNode>>,
}

/// 请求级不可变配置（`chat_request::Request` 的取值快照，全程只读）。
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
    pub(crate) auto_compress: bool,
    pub(crate) load_history: bool,
    /// 请求显式给的系统提示词（`None` = 只有注册段；不吞掉注册段，见
    /// `chat_loop::inputs::resolve_system_prompt`）
    pub(crate) system_prompt: Option<String>,
}

impl TurnRequest {
    /// `auto_compress_default` 来自本轮配置快照（`TurnSettings.auto_compress`）——
    /// 默认值的唯一真源仍是配置层，本结构只是它的**请求级投影**。
    pub(crate) fn new(req: &chat_request::Request, auto_compress_default: bool) -> Self {
        Self {
            auto_compress: req.auto_compress.unwrap_or(auto_compress_default),
            // 请求级默认（该字段无配置对应项），语义为"缺省即加载历史"。
            load_history: req.load_history.unwrap_or(true),
            system_prompt: req.system_prompt.clone(),
        }
    }
}

/// 轮次状态：请求作用域内的可变状态。
///
/// 收口前这些量散落在 `run_chat_loop` 的循环作用域里，并以 6 个 `&mut` 参数逐个
/// 穿进收尾函数；现在收成一个结构体，主循环与阶段函数共享同一份状态。
#[derive(Default)]
pub(crate) struct TurnState {
    /// 用户中止信号（`provider.round_trip` 与工具执行共享同一份）。
    ///
    /// 收口前是一个裸 `Arc<AtomicBool>`，外部置位要靠「往执行期通道投 Abort 帧」；
    /// 现在是 [`ExecAbortSignal`]——置位与唤醒是同一个动作，不再需要帧与轮询。
    pub(crate) abort: ExecAbortSignal,
    /// 本轮**已发出的 LLM 请求数**，也就是「这是第几幕」（`D-222` §二：幕是环的身份）。
    ///
    /// 唯一写方是主循环的**幕间**那一步，而它的取值来自事实
    /// （`loop_rows::next_stage_round` 读 `turn_head.round` 给出下一幕的轮次）——
    /// 不是一份自己往上加的计数。续写次数因此不再需要独立的内存量
    /// （`continuation_count` 已随幕的迭代器退场，`D-222` §二），一次请求加一格，
    /// 绝大多数轮就是 0 或 1。
    ///
    /// ⚠️ 它**不是**工具轮数。v2 的工具循环在 `turn_exec::drive` 的环体里
    /// 一批批跑完，那里有唯一的真实计数（`tool_round`）；本层一次请求就是一个整体，
    /// 看不到第几轮工具。于是这一层的判据只能是「这是不是本轮**第一次请求**」——
    /// 轮首判决、长期记忆召回、`user_prompt_submit` 钩子、流式起始的轮次标注都是它。
    /// 把请求数当工具轮数用会**静默失效**（判据恒假而默认值看起来像「没配」），
    /// 那条病灶与它三个读方各自的去处见
    /// [D-074](../../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")。
    pub(crate) request_round: usize,
    /// 增量落库锚点：本轮已落库到的下标（每轮开始时重置为本轮起始消息数）
    pub(crate) last_saved: usize,
    /// 本轮**输入**（消息 id + 正文），在 `single_message` 被消费之前锚定。
    ///
    /// 锚定而不现取，是因为 `context.messages` 装着**整段历史**（`load_history = true`
    /// 时每轮都重新加载）：从里面找「本轮用户发言」找到的永远是首轮那句。消费方是
    /// 判决（`first_utterance`）与委派段的 Q1（`inputs.rs`）——共用同一份锚，
    /// 两处才不会各读各的。
    ///
    /// `None` = 本请求没有用户新发言（`resume` 重跑等）——判决无从谈起，
    /// Q1 也不下结论（见 `delegate.rs`）。
    pub(crate) input_utterance: Option<(String, String)>,
    /// 本轮的**长期记忆召回视图**（`memory_events::recall_view`）。
    ///
    /// 只在本轮**第一次请求**取一次（与 classify / history 同一条 `request_round == 0`
    /// 口径）：同一轮内记忆不该漂移，事实源也只扫一遍；后续工具轮复用同一份视图，
    /// 渲染 [`chat_loop::inputs`] 每次调用现算（纯内存，零 I/O）。
    ///
    /// 落 `memory.recalled` 的时机在**轮末收束**（`derived_facts::record_learning`）而不是
    /// 取视图那一刻：溯源锚是本轮 `user.message` 格，那时它才在事实源里。
    pub recall_view: Option<super::super::recall::RecallView>,
    /// 本轮**已收集的上下文段**（DE-025 D-215）：由 `run_recall` 一次收集、本轮各
    /// 工具轮复用。
    ///
    /// 与 [`TurnState::recall_view`] 同一条取用口径（`request_round == 0` 取一次、
    /// 本轮各工具轮复用）：同一轮内上下文不该漂移，也省得每个工具轮都重跑一遍
    /// provider。它**只读不写**——某段算不出来（没召回 / 就绪集空）就是没有那一段，
    /// 请求视图里少一段，不存在「空占位」。
    ///
    /// 为什么是「一整份段」而不是逐段一个字段：段的数量由 provider 决定，本轮代码
    /// 不该知道有哪几段——逐段字段就是「加一类上下文要改 `TurnState`」。
    pub(crate) context_sections: Vec<symbio_plugin_api::ContextSection>,
    /// 本轮**对话面开场白**正文：`Escalate` 首响。
    ///
    /// 由轮首判决执行体的 `apply_verdict`（`VerdictEffect::Spoke`）填充，交 `turn_exec::drive`
    /// 落成 `chat.assistant.opening` 格。**落格不在本文件**——写者令牌只在执行口，
    /// 与答话同一条分割（判定与措辞在 chat_loop，落格在 turn_exec）。
    /// `None` = 本轮没有开场白（常态）。
    pub(crate) opening_text: Option<String>,
    /// 开场白**对话面节点**的身份与标记（仅 [`TurnState::opening_text`] 有值时消费）。
    ///
    /// 三样（`id` / `reason` / `exclude_from_context`）是节点形状的一部分，必须随
    /// 事实入格，否则重建折出的节点与实时面是两个身份（见 `TurnInput::opening_dialog`）。
    pub(crate) opening_dialog: Option<super::super::turn_runner::DialogFacts>,
    /// **本幕判决判成的定稿答话**（`policy.classify` 执行体的产出，D-162 裁决 3）。
    ///
    /// 判决执行体在轮首把这一轮判成「不用再发请求」时，把答话正文连同对话面事实寄
    /// 存在这里：执行体不交回产出（D-123 决策 5），而收束格要由**派发点**在两个动作
    /// 之间落——只有派发点知道判决是否已经问完。`None` = 本轮照常进工具循环。
    pub(crate) head_answer: Option<super::super::chat_loop::AnsweredTurn>,
    /// 本轮请求的**固定开销**（系统提示词 + 工具清单折算的 token 数）。
    ///
    /// 由准备段（`collect_reads`）算出后写在这里，供压缩行的执行体
    /// 判水位——寄存一个数，而不是把「收集提示词与工具」搬进执行体（那是收口 ②
    /// 的唯一收集点，拆两处就出现两个时机）。0 = 本轮还没收集过（压缩行不会被选中）。
    pub(crate) request_overhead: usize,
    /// 本次执行的轮号（新轮 = WAL 内既有用户格计数；续写幕 = 被续写轮号）。
    ///
    /// 由开轮块（`chat_loop` 步骤 2a）在**任何判决之前**定下：用户格先落，
    /// 运行器不再自己数。
    pub(crate) turn_no: u64,
    /// 本轮的**溯源锚**（用户格 seq；续写幕 = 原轮用户格 seq）。
    pub(crate) user_seq: u64,
    /// **本幕的锚**：新轮 = 用户格 seq；续写幕 = 恢复阶段事实 seq。
    ///
    /// `turn_head` 只看这个号之后的阶段事实——同一轮号会被续写幕复用，
    /// 按轮扫全史会把首幕的格误判成「已做」。
    pub(crate) drive_seq: u64,
    /// 本幕的 `now`：**唤醒事实的 `ts`**（判据 12 / DE-015 §2.1）。
    ///
    /// 幕内所有 `owed` / `next` / 带时间的视图都用它，不再各读一次墙钟——否则
    /// 同一个日志重放两次会走两条路。幕首由 [`super::drive::begin_drive`] 从刚
    /// 落格的那条事实读回；0 = 尚未开幕（与 `turn_no` 的默认值同义）。
    pub(crate) now_ms: i64,
    /// 本幕读到的事实切片（阶段「已做」与配置快照的唯一输入）。
    pub(crate) facts: Vec<symbio_core::Event>,
    /// 续写幕复用的 Turn 根节点 id（d）。
    ///
    /// 唤醒请求发生在开幕之前，幕首（`turn_exec::drive` 的 `open_stage`）按它决定
    /// 「要不要新开一个根」：
    /// 有值 ⇒ 复用被续写轮的根（结果卡挂回原来那棵树），并且**不重发**流式占位
    /// ——那个骨架早在本轮首幕就发过了。`None` = 新轮，照常新开。
    pub(crate) resume_root: Option<String>,
    /// 续写幕要**替换**的旧结果子节点（失败结果 / 等待用户的结果）。
    ///
    /// 新结果随执行器交回，落库之前把旧的删掉：留着会让同一笔调用出现两条结果，
    /// 重建面（只按 `artifact.added` 复刻新结果）与实时面因此各说一套。删除放在
    /// 派发**成功之后**（`chat_loop` 的并入处），派发失败时旧卡仍在。
    pub(crate) resume_old_child: Option<String>,
    /// `RetrySubtree` 交回来的**被重试那一支的 Turn 根 id**（其余动作为 `None`）。
    ///
    /// 它是「这一幕是新支」的唯一记号：`prepare-turn` 备好输入、因而握着新根 id 时，
    /// 按它落一条 `turn.branch`（`{of, into}`）。旧支当场从「当前读的那一支」降成候选——
    /// 实时面由收尾按那一格补发标注帧，不必等整份重读。
    /// 取走（`take()`）即消费：幕间续写的第二幕不再另开一支。
    pub(crate) retry_of: Option<String>,
}

impl TurnState {
    /// 本轮开出的**新支根 id**：本幕 `turn.branch` 那一格的 `into`（没有 = 本轮没开新支）。
    ///
    /// 取最近一格：幂等键是 `branch-{root}`，一轮正常最多一条；按 `turn` 收窄排除了
    /// 上一轮的支。读的是本幕已经在手的事实切片（`self.facts`），不为补发再多读一次日志。
    /// 它**不是**一份状态：这就是那一格事实本身，草稿不另存一个 `branch_root` 字段
    /// （DE-022 D-226）。
    pub(crate) fn branch_root(&self) -> Option<String> {
        self.facts
            .iter()
            .filter(|e| {
                e.turn == self.turn_no && e.kind == symbio_plugin_api::events::EVENT_TURN_BRANCH
            })
            .filter_map(|e| e.payload.get("into").and_then(|v| v.as_str()))
            .next_back()
            .map(str::to_string)
    }
}

/// 主循环的唯一退出原因。
///
/// 每个出口只负责**判定原因**；收尾（提示文案 + 增量落库 + Stop 钩子 + 返回语义）
/// 统一由 [`finish_turn`] 执行——新增出口不必再记得补齐三件套。
#[derive(Debug)]
pub(crate) enum TurnExit {
    /// 正常收尾：无工具调用 / 工具待用户输入。
    Completed,
    /// 循环顶部的中止检查点：上一轮 Turn 已定稿落库，**不冒泡**中止
    /// （否则消费循环的 `persist_failure` 会把已成功的 Turn 误标为 `Aborted`）。
    AbortedAtBoundary,
    /// 用户中止且**在途 Turn 尚未定稿**：冒泡中止，由消费循环落库为
    /// `NodeStatus::Aborted` + 会话结局 `aborted`（**不是** `failed`），
    /// 前端渲染错误条与重试入口。携带**半截产出**（正文 / 思考 / 工具节点）——
    /// 它们是事实，随收尾入格（`persist_failure` 的 `partial`）。
    Aborted(Vec<ChatNode>),
    /// LLM / 编排失败：冒泡原始错误。
    Failed(PluginError),
    /// resume 已完成，无需进入主循环。
    ResumeDone,
}

/// 主循环顶部的唯一闸门：**退出条件**。
///
/// 形状原本是三态（放行 / 等工具 / 退出），"等工具"那一态随 v1 外壳的簿记一起坍缩：
/// 见 [`gate_turn`] 的文档。
#[derive(Debug)]
pub(crate) enum Gate {
    /// 条件齐备 → 进入本轮准备与推理。
    Proceed,
    /// 必须退出，携带原因。
    Exit(TurnExit),
}

/// ## 为什么没有 `new()`
///
/// 字段全是 `pub`，构造点是**唯一**的（`orchestrator/consume.rs`）。八参的
/// `new()` 是八个**位置参数**，读调用点要靠数数才知道谁是谁，每加一个字段就要
/// 再挤一个（`clippy::too_many_arguments` 在第八个就报错）。
/// 结构体字面量把"谁是谁"写在字段名上，**字段增删由编译器在唯一构造点报错**，
/// 比位置参数强。这里不再补 `new()`。
pub struct ChatOrchestrator {
    /// 唯一生效的模型服务（model 插件按上下文解析后经 CAPABILITY_VISITOR 注册；
    /// core 纯 trait 的 trait object——loop 对协议实现零依赖）
    pub provider: Arc<dyn ModelProvider>,
    pub parent: Option<Arc<dyn Plugin>>,
    /// 预计算的生效上下文上限（`provider.effective_context_tokens()` 结果，
    /// session 在派发前已解析）。
    pub context_limit: u32,
    /// 本轮会话句柄：纯值输入 + 事实轨 + 动作面端口（见 `LoopSession`）。
    ///
    /// 转写、压缩熔断、用户补充队列与配置当前值都在 session 墙内，这里只经端口读写——
    /// 驱动器不持 `PersistentChatSession`（E-009）。
    pub session: Arc<super::super::turn_session::LoopSession>,
    /// 压缩增量改道用的转写写口（`TurnDispatch.writer`：session 的转写唯一写入点）。
    pub writer: Arc<dyn symbio_core::ExecTranscriptWriter>,
    /// 对话面措辞的**请求级开关快照**（`SessionConfig::compose_enabled`）。
    ///
    /// ## 为什么也是一个值
    ///
    /// 与判决开关（`session.config` 事实）同一条取值纪律：配置在一次请求生命周期内
    /// 不变，措辞只需要一个布尔。持有 `Arc<SessionPlugin>` 会多一条绕过配置面的路径，
    /// 而这里没有任何"当前值"要读。
    ///
    /// ## 它关掉的是什么
    ///
    /// `false` ⇒ 本循环**不调用** `compose/wording`：`Answered` 拿不到措辞，
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
    /// 对话面开场白（`Escalate` 首响）的**请求级开关快照**（`SessionConfig::opening_enabled`）。
    ///
    /// 与 `classify_enabled` / `compose_enabled` 同一条取值纪律（一次请求生命周期内不变，
    /// 取一个布尔）。`false` ⇒ [`super::compose::apply_verdict`] 的 `Escalate` 分支**不写**
    /// 那句开场白（本轮照旧进工具循环），真实答复（`Answered`）不受影响。为什么独立于
    /// `compose_enabled`：后者关掉会连答复一起没，本开关只削派活前的旁白。
    pub opening_enabled: bool,
    /// 建树策略的**请求级快照**（`TurnSettings::presentation_tree`）。
    ///
    /// 与 `classify_enabled` / `compose_enabled` 同一条取值纪律：组树档在一次请求生命周期
    /// 内不变，取一次快照即可。`flat` 下本轮响应骨架的直接子节点升根（见
    /// [`symbio_core::node_build::detach_if_flat`]）；前端只按 `parent_id` 插树、不看本字段。
    pub presentation_tree: symbio_core::node_build::PresentationTree,
}

impl ChatOrchestrator {
    pub async fn finalize_assistant_turn(
        &self,
        root_id: &str,
        out: &TurnOutput,
        sink: &ExecEventSink,
    ) {
        let principal = self.session.principal();
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
            // 流式期间 ReasoningDelta 已经把 reasoning 逐帧上线进 reasoning_child_id 节点，
            // 这里只迁**状态与类型**（状态帧不带正文，重发一遍只是二次传输同一段文字）。
            //
            // 类型必须是 **Text**：落库侧把这一段承载成 Text 子节点（复用同一个流式 id），
            // 网格重建同判据 ⇒ 重载会话里它是「回答」。流式层若仍标 Reasoning，
            // 同一条消息在 live 是折叠的思考块、刷新后是正文——「模型只想不作答」
            // 就是这一处错位造成的观感，不是模型的行为。
            if !out.reasoning_child_id.is_empty() {
                let mut node = ChatNode {
                    id: out.reasoning_child_id.clone(),
                    parent_id: Some(root_id.into()),
                    principal: Some(principal.clone()),
                    msg_type: Some(NodeKind::Body),
                    // **换形态要带清除格**：s3 起思考节点的呈现身份住在 `meta.style_tag`，
                    // 帧的合入是「有则覆盖」——不带 meta = 旧 tag 留在消费方快照里，
                    // `is_text` 永远折叠不出正文（CLI 不回显、以失败退出；e2e t55/t77 抓到）。
                    // 空对象 = 整格覆盖且无 tag，把「同 id 换类型」说完整。
                    meta: Some(serde_json::json!({})),
                    content: Some(MessageContent::Text(out.reasoning.clone())),
                    status: Some(NodeStatus::Completed),
                    ..Default::default()
                };
                detach_if_flat(&mut node, root_id, self.presentation_tree);
                llm_emit_state(sink, node).await;
            } else {
                let resp_id = if out.response_text_child_id.is_empty() {
                    llm_short_id()
                } else {
                    out.response_text_child_id.clone()
                };
                // 兜底路径：这个 Text 节点是**新建**的（reasoning-only 时没有正文
                // 子节点），正文必须随帧上线 ⇒ 完整消息帧。
                let mut node = ChatNode {
                    id: resp_id,
                    parent_id: Some(root_id.into()),
                    principal: Some(principal.clone()),
                    msg_type: Some(NodeKind::Body),
                    content: Some(MessageContent::Text(out.reasoning.clone())),
                    status: Some(NodeStatus::Completed),
                    ..Default::default()
                };
                detach_if_flat(&mut node, root_id, self.presentation_tree);
                llm_emit_message(sink, node).await;
            }
            // 根 Turn 的终态**不在此发出**：Turn 是组合节点（仅分组，无正文），
            // 终态必须晚于子树——而本轮的 ToolCall 要到派出那一环步里才执行完。
            // 唯一发射点是 `run_chat_loop` 的 `finalize_turn_root`（本轮收尾点）。
            return;
        }

        // Mark reasoning child as completed
        //
        // 「思考就是这一轮的回答」有两种到达形态：`text` 空（v1 的 reasoning-only），
        // 与 `text` 已被运行器**取作**那段思考（v2 收束格的 `text` 存的是回答，
        // 回答取自思考时两者逐字相同）。落库侧与网格重建按**内容相同**这条判据
        // 把它承载成正文子节点（`llm_build_assistant_messages` 的 `reasoning_only`），
        // 这里必须跟着同一个判据——否则 live 标推理、存储标正文，同 id 两个类型，
        // 界面上就是「有思考没正文」。
        if !out.reasoning.is_empty() && !out.reasoning_child_id.is_empty() {
            let answer_is_reasoning = out.text.trim() == out.reasoning.trim();
            let mut finalized = ChatNode {
                id: out.reasoning_child_id.clone(),
                parent_id: Some(root_id.into()),
                principal: Some(principal.clone()),
                msg_type: Some(NodeKind::Body),
                // 与上面 reasoning-only 分支同一纪律：答案取自思考时要把 tag 清除格
                // 说完整（`with_style_tag` 在未覆盖形态下重新打回 reasoning，两分支
                // 各自的最终 meta 都由这一格决定）。
                meta: Some(serde_json::json!({})),
                content: Some(MessageContent::Text(out.reasoning.clone())),
                status: Some(NodeStatus::Completed),
                ..Default::default()
            };
            detach_if_flat(&mut finalized, root_id, self.presentation_tree);
            llm_emit_state(
                sink,
                if answer_is_reasoning {
                    // 答案取自思考：这一格就是正文（`Body`），不挂 reasoning tag。
                    finalized
                } else {
                    // 独立思考：归位 `Notice`（折叠叶），呈现（title/icon）
                    // 由前后端交互面统一派生，不在这里写；tag 只作后端业务注记。
                    let mut n = finalized;
                    n.msg_type = Some(NodeKind::Notice);
                    n.with_style_tag(CHAT_STYLE_TAG_REASONING)
                },
            )
            .await;
        }

        // Mark response text child as completed (exists if there was text content)
        if !out.text.is_empty() && !out.response_text_child_id.is_empty() {
            let mut node = ChatNode {
                id: out.response_text_child_id.clone(),
                parent_id: Some(root_id.into()),
                principal: Some(principal.clone()),
                msg_type: Some(NodeKind::Body),
                content: Some(MessageContent::Text(out.text.clone())),
                status: Some(NodeStatus::Completed),
                ..Default::default()
            };
            detach_if_flat(&mut node, root_id, self.presentation_tree);
            llm_emit_state(sink, node).await;
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
        // `tool_executor::process_tool_calls_async`（执行前 `Streaming`，执行后
        // `Completed` / `WaitingUserAction`；未执行的批尾统一收口）。审批恢复不再
        // 另走一条执行路径——它把用户动作落成唤醒请求，仍由同一执行方收口。
        //
        // 状态机见 `crates/symbio-plugin-session/src/docs/105-node-state-streaming.md` §2.3；
        // 「每个 ToolCall 必然到达终态」这条不变量由这一处负责保证。

        // 根 Turn 的终态**不在此发出**。
        //
        // LLM 流结束只说明「模型这一轮说完了」，不说明「这一轮结束了」：Turn 是
        // **组合节点**（仅分组、无正文），它的终态必须**跟随子树**——而本轮的
        // ToolCall 此时连执行都还没开始（执行在 v2 运行器内部，含整个执行窗口）。
        // 若在此标 `Completed`，树上就会出现"容器已完成、其中的工具调用仍在运行"
        // 这种自相矛盾的形状（实测抓包 seq 5 早于 seq 6–8 即此）。
        //
        // 唯一的发射点是派出那一环步在 `persist_turn` 之后调用的
        // [`finalize_turn_root`]：那一刻子树才真正收敛，且发出的正是**即将落库的
        // 那条节点**——live 与 storage 按同一取值收敛。
    }
}

#[cfg(test)]
#[path = "state.test.rs"]
mod tests;
