//! 事件契约 —— v2 事件溯源的共享类型层（[plan/01 §2 / §6](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 为什么是独立模块
//!
//! 施工视图的六个包（`actors` / `projections` / `store` / `adapters` / `governance`）
//! 彼此**禁止互相依赖**，唯一例外是「事件与 View 的类型定义」——那是共享契约
//! （[plan/05 §3.1](../../../../docs/plan/05-模块架构.md)，01 §13.5「契约从边上挪到中心」）。
//! 因此事件类型必须住在**中性层**：`store`（④）与 `projection`（③）都只 import 本模块，
//! 谁也拿不到谁的句柄。
//!
//! ## 冻结锚点（[plan/03 §1](../../../../docs/plan/03-演进与验证.md)）
//!
//! - **F5**：动词 5 个、能力 7 个、模式 3 个 —— 本模块冻结动词与实体的**闭集**
//!   （`Verb` / `Entity` 枚举）。新增格子 = `match` 加一臂，是数据变化不是机制变化；
//!   枚举而不是字符串，是为了让"网格之外的事件"**编译不过**（J3）。
//! - 事件**语法网格**（01 §6）：`实体 × 动词` 的格子上有若干**名字**（如
//!   `chat.assistant.final` 落在 `turn × closed`）。名字是数据，加名字不加枚举。

use serde::{Deserialize, Serialize};

/// 不可伪造的单调序号（[plan/01 §2](../../../../docs/plan/01-核心架构.md)）。
///
/// 字段私有：**只有 `Store` 能构造**（`pub(crate)`），调用方只能拿到与比较——
/// 自己 `Seq(5)` 伪造一个序号编译不过。这就是「类型强制单调」的构造侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Seq(u64);

impl Seq {
    /// 仅 crate 内（`Store` 实现）可构造。
    pub(crate) fn new(v: u64) -> Self {
        Seq(v)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

/// 事件时间戳：Unix 毫秒（UTC），口径同 [`crate::symbio_core::clock_now_ms`]。
pub type Timestamp = i64;

// ── 事件名字表（S01：turn/* 全部，[roadmap/S01 §3](../../../../docs/stage/S01-最小闭环.md)）────
//
// 名字是**数据**（加名字不加枚举，01 §6）；但名字必须**单点定义**——散在调用点的
// 字符串字面量必然漂移（同 merge_supplements 的单点判据）。常量只钉名字，
// 网格坐标（落在哪个 entity × verb 格子）由构造方用枚举保证。

/// 用户发言。落在 `turn × opened`。
pub const EVENT_USER_MESSAGE: &str = "user.message";
/// **轮边界折进的补充**（载荷 `{ text, count }`）。落在 `turn × asserted`。
///
/// ## 为什么它必须是一格事实（缺口 3）
///
/// 用户在助手干活途中补了一句话，那句话被折进 `context.messages` —— 但 `TurnInput.text`
/// 在**轮首就定死了**，于是这句话：
///
/// - 进了消息列表（会落库、会进 `messages.json`），
/// - **没进事实网格**，而 prompt 是从事实网格投影的 ⇒ **模型看不见它**，
/// - 且**完全静默**：不报错、不告警、既有断言全绿。
///
/// 「会话接受了一句用户话，但它从未成为事实」——补一句成为事实，最小的格子就是它。
///
/// ## 为什么不是 `user.message` 的第二次
///
/// `user.message` 落在 `turn × opened`，语义是「开轮」。轮已经开了（补充发生在
/// 干活途中），再落一条 `opened` 会让「一个 turn 一个开轮格」这条不变量失效。
///
/// ## `count`：一次可能折进多条
///
/// 抽干是**批量**的（`merge_supplements` 把同窗口内的多条合并成一条消息）。
/// `text` 是合并后的正文，`count` 是**原条数**——记下来是因为「模型收到一大段」
/// 与「用户连说了 5 句」对模型是不同的输入，而网格不该只保留前者。
pub const EVENT_TURN_SUPPLEMENTED: &str = "turn.supplemented";

/// **轮边界汇报**（载荷 `{ text, tool_rounds, quiet_ms }`）。落在 `turn × asserted`。
///
/// ## 它为什么是事实（缺口 4）
///
/// 「干到一半也要能说一句话」。那句话由 `compose` 的模板产线生成（零 LLM 往返），
/// 不能只靠 chat_loop 的轮边界发——工具循环在 v2 运行器里，轮边界每个用户轮只经过
/// 一次、且判定头一句就是 `tool_rounds == 0 ⇒ 不说`（那个计数只在工具循环里递增）
/// ⇒ **恒不发**。
///
/// 汇报不是轮次事实（一轮可以报零到多次），所以它与 `turn.supplemented` 同格
/// （`turn × asserted`），不占 `chat.assistant.final` 那格 N3 的「每轮至多一条」。
///
/// ## 为什么标注「不进请求包」
///
/// 汇报是**界面文本**：`exclude_from_context`（与 `Escalate` 首响同一条纪律）。
/// 投影仍然收它 —— 用户看得见、落库要留住；只是 `to_messages()` 把它滤掉。
/// 滤掉这件事由 `prompt_fidelity` 反向盯着：它一旦漏进请求包就报缺口。
pub const EVENT_ASSISTANT_REPORTED: &str = "chat.assistant.reported";

/// **对话面开场白**（载荷 `{ text, id, reason, exclude_from_context }`）。落在 `turn × asserted`。
///
/// ## 它为什么是事实（S2.7）
///
/// `Escalate` 判决在轮首产出一句首响（「好，我来处理」），随后本轮**继续**进工具循环
/// ——它与 `chat.assistant.final` 不是同一条：收束格是干活干完之后的答复，本格干活
/// **开始前**的开场白。双写期这句随整份 `messages.json` 落盘，切源（WAL 为唯一真源）
/// 后若不入格，重建便折不出这条根级对话面节点——实测 t20 / t22 的「恰有一条对话面
/// 节点」全部落空（`meta.surface = "reply"` / `meta.reason = needs_work`）。
/// 节点形状是事实的一部分（S2.6 原则），故单立一格。
///
/// ## 为什么与 `chat.assistant.reported` 分格而不是复用
///
/// 汇报的理由码恒为 `progress`（界面进度），本格的理由码是上游判决原样透传
/// （`needs_work` 等）；两者的 `exclude_from_context` 虽同为真（都是界面文本、
/// 不进模型请求包），但**理由码是数据**，把「开场白」塞进「汇报」那格会让
/// `session/stats` / 前端面板把开场白读成进度汇报。名字是数据，加名字不加枚举。
///
/// ## 为什么标注「不进请求包」
///
/// 与 [`EVENT_ASSISTANT_REPORTED`] 同一条纪律：开场白是**界面文本**，
/// `exclude_from_context = true`。投影仍然收它（用户看得见、落库要留住），
/// 只是 `to_messages()` 把它滤掉——由 `prompt_fidelity` 反向盯着。
pub const EVENT_ASSISTANT_OPENING: &str = "chat.assistant.opening";

/// 助手最终答复（每 turn 至多 1 条——N3）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FINAL: &str = "chat.assistant.final";
/// 兜底话术——**普通事件，不是特殊通道**（生成失败也要有输出）。落在 `turn × closed`。
pub const EVENT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";

/// 等待用户（本轮尚未了结，如工具报了 confirm / ask_user）。落在 `turn × progressed`。
///
/// ## 为什么它是事实而不是「什么都没发生」
///
/// 思考 / 正文 / 工具是**事实**；完成 / 终止 / 等待是事实的**状态**。等待用户时模型
/// 已经说过话（"需要你确认一下"）——那句话是事实，必须入格（载荷 `status` 记
/// `pending`），否则恢复续写后模型看不见自己问过什么，会把同一句再问一遍。
/// **不为「等待」新增收束语义**：本格不收束轮次（`turn × progressed` 不是
/// `closed`），恢复后照常以收束格了结，「一轮至多一条收束格」不变。
///
/// ## 为什么不是 `chat.assistant.reported` 的第二次
///
/// 汇报是**界面文本**（`exclude_from_context`，不进请求包），而等待用户那句话是
/// **对话事实**——恢复续写时模型必须看得见它。两者角色相同（assistant）、
/// 进不进请求包相反，混在一处必然让其中一半被静默滤掉。
pub const EVENT_ASSISTANT_PROGRESS: &str = "chat.assistant.progress";

// ── 产物格子（S02 §3，[roadmap/S02-工具调用与产物.md](../../../../docs/stage/S02-工具调用与产物.md)）──
//
// 工具**结果**是产物事实：模型请求了工具、工具产出了这段东西。工具调用**本身**
// 不单独占格（同一件事两格必然漂移）；`produced_by` 指向**本轮用户格**，即
// 「这个产物是本轮用户请求的后果」——S02 §3 的验收断言正是 `caused_by` 指向发起
// 它的 `task/turn`。

/// 产物产出：一次工具调用的**结果**（载荷 `{ tool, text }`）。
/// 落在 `artifact × asserted`（网格坐标见 [plan/01 §6](../../../../docs/plan/01-核心架构.md)）。
pub const EVENT_ARTIFACT_ADDED: &str = "artifact.added";

// ── 消息列表变更的格子（S2.7，ADR-053 / ADR-054）──
//
// 「消息列表被改过」也是**发生过什么**。在轨 S 之前，编辑 / 截断只重写
// `messages.json`，事实源里一个字都没有 ⇒ 主读路径一旦切到「从 WAL 重建」，
// **被截断的消息会复活、编辑过的内容会回退**。
//
// 这一族是**闭集**（ADR-054 决策 1）——列表的改动只有四种，三个节点级 + 一个列表级：
// 诞生（本格）/ 断言（`message.edited`）/ 关闭（`message.removed`）/ 取代
// （`session.consolidated`）。任何场景都是它们的组合；**新场景若需要第五种 ⇒ 模型错了**，
// 应当改模型而不是加一格。
//
// ## 为什么是 `message` 实体而不是 `turn`
//
// §6 已预留 `user.message.edited`（`turn × opened`），那是「用户改刚发的那句话」
// 这一**窄义**。而 CRUD 是**列表级、跨轮**的：一次截断可以横跨多轮，归 `turn`
// 会把「一轮开始」与「一段历史被删」压进同一格（准入三问「能归入已有实体？」的
// 否定一侧）。

/// 一条消息**诞生**（载荷 `{ messages: [<ChatMessage JSON>…] }`）。落在 `message × opened`。
///
/// ## 为什么需要它（`turn` / `artifact` 不是已经承载了节点吗）
///
/// 大部分节点的诞生**由别的事实承载**：用户发言归 `turn × opened`、助手正文归
/// `turn × closed|progressed`、工具结果归 `artifact × asserted`（重建侧的
/// `nodes_of` 就是从这些格取出节点的）。但有一类节点**没有别的格可搭车**——
/// 压缩记录节点（「已压缩上下文」那张卡）是最典型的一个：它是一次压缩的**记录**，
/// 不是任何一轮的产出，也不属于任何工具调用。
///
/// 少了这一格，那类节点在事实源里**从来没有出生过** ⇒ 切主读路径之后它们整条消失
/// （ADR-054 决策 1 的「诞生」原语）。
///
/// ## 边界：它是「没有语义承载」的那一格
///
/// 判据是**否定的**——只有当一条节点的诞生**找不到别的格**时才用它。承载着对话内容的
/// 节点（用户发言 / 助手正文 / 工具结果）必须由它们自己的事实落，否则同一条节点
/// 有两个 owner（ADR-049 理由 1）。
/// 因此本格的节点是**界面文本**类（`prompt_fidelity` 的 `excluded_from_prompt`），
/// 不进模型请求包。
///
/// ## 为什么载荷是列表而不是单条
///
/// 与 `message.removed` 的 `ids` 同一条判据：**一个动作一条事实**。一次「诞生若干节点」
/// 的操作（压缩收口时把记录节点连同收口节点一起写回）落一条，而不是 n 条。
pub const EVENT_MESSAGE_ADDED: &str = "message.added";
/// 编辑一条消息（载荷 `{ message: <ChatMessage 的 JSON> }`——补丁**应用后**的整条）。
/// 落在 `message × asserted`。
///
/// ## 为什么载荷是整条而不是字段子集
///
/// 重建与转写都按 **id 替换**：拿到整条就不必再猜「这次改的是哪几个字段」——
/// 补丁语义本来就是「未提供的保持不变」，而应用后的整条**就是**那个结果。
/// 只记子集等于把「合并」这件事留一份实现给重建，两份必然漂移。
///
/// ## 为什么它必须带 `produced_by`
///
/// 它是断言类（I2 无溯源不声明），且溯源有确切的答案：**被改的那条事实**
/// （`produced_by` = 承载该节点的那条事件的 seq）。「这次编辑源自那条发言」——
/// 而不是拿一个「此刻」的号凑数。
pub const EVENT_MESSAGE_EDITED: &str = "message.edited";
/// 一段消息没了（载荷 `{ ids: [...] }`）。落在 `message × closed`。
///
/// ## 为什么一次操作只落一条（区间用 id 列表表达）
///
/// 截断的语义是「从 X 起全没了」——**一个动作**。逐条发事件会把一个动作拆成 n 条
/// 事实，n 随消息量增长，而事实网格的价值正在于「一件事一个键」。与
/// `memory.forgotten` 同形：收束格表达「没了」，载荷说清「哪些」。
///
/// `ids` 是**被删节点 id 的完整列表**（与 `truncate_messages` 回执里那份同源）——
/// 重建与转写据此丢弃节点，不做「从某条起」的区间推断（区间推断要求重建知道
/// 列表顺序，而顺序是呈现细节，不是事实）。
pub const EVENT_MESSAGE_REMOVED: &str = "message.removed";

/// 会话历史的**压缩快照**（载荷 `{ messages: [<ChatMessage JSON>…] }`——压缩后的整份列表）。
/// 落在 `message × progressed`。
///
/// ## 为什么是 `message × progressed` 而不是 `system`
///
/// 用 ADR-053 自己的判据：双源对账
/// （`session.reconciled`，已随 S2.7-a 退役）曾归 `system`，理由是「它说的不是
/// 『这个会话发生了什么』，而是『两个读源此刻是否一致』」——诊断。而压缩**确实发生过**：
/// 一段历史被摘要取代、列表从此读作快照那一份。那是
/// **会话事实**，故归 `message`（本实体正是「消息列表上的变更」）。动词取 `progressed`
/// 与 `memory.consolidated` 同——两者都是「巩固」。
///
/// ## 它取代前缀：live 段就是全史
///
/// 一条 `session.consolidated` **取代**它之前的所有消息事实：此后所有读侧
/// （`rebuild` 折节点树、`transcript` 折对话行、`stats` 扫水位…）看到的「历史」
/// 都是「最近快照 + 其后事实」。被取代的那一段**不删**——它移入归档段
/// （`WalStore::archive_before`），留作审计与回溯。**不删是因为事实不可凭空消失**；
/// 移出 live 段是因为「压缩」的语义就是「那段历史已被摘要取代」，而 live 段
/// 承载的是**当前有效的历史**。
///
/// 于是「WAL 无界增长」与「上下文压缩」由**同一个机制**解决（ADR-053 决策 6）：
/// 一次压缩 = 多一条快照事件 + 把被取代的前缀移入归档段。
pub const EVENT_SESSION_CONSOLIDATED: &str = "session.consolidated";

// ── 记忆格子（S5，[roadmap/S06 §3](../../../../docs/stage/S06-长期记忆与语义检索.md)）────

/// 编码：学到的语义内容 + embedding（`payload: { content, tag, generation, vec }`）。
/// 落在 `memory × opened`。**记忆必带溯源**（I2 扩展：溯源覆盖 100%）。
/// `vec` 是向量索引的载荷位：**索引未建时写 `[]`**——内容恒在 `content` 里，
/// 索引落成时按 `content` 回填即可（写方 `plugins/session/v2_memory.rs`）。
pub const EVENT_MEMORY_ENCODED: &str = "memory.encoded";
/// 巩固：压缩 + 反事实（不是逐帧回放）。落在 `memory × progressed`。
pub const EVENT_MEMORY_CONSOLIDATED: &str = "memory.consolidated";
/// 遗忘：**不是物理删除**——Log 永不删，只是投影不再包含（可撤销、可审计）。
/// 落在 `memory × closed`。
pub const EVENT_MEMORY_FORGOTTEN: &str = "memory.forgotten";
/// 召回：检索 Translator 的产出（读视图 → 产出事实）。落在 `memory × asserted`。
pub const EVENT_MEMORY_RECALLED: &str = "memory.recalled";

// ── 承诺格子（S6 第 15 步，[roadmap/S08 §3](../../../../docs/stage/S08-多主体与对等承诺.md)）──
//
// 立约 = `commitment` 实体的一格事件（**加格子，不加机制**——对等协作是图，
// 子智能体递归的树表达不了，才需要这组格子）。声誉是它的投影（`projection = reputation`）。

/// 立约：`from` 向 `to` 承诺交付什么（载荷 `{ id, from, to, promise }`）。
/// 落在 `commitment × opened`。
pub const EVENT_COMMITMENT_OFFERED: &str = "commitment.opened";
/// 守约收束：承诺按约履行。落在 `commitment × closed`。
pub const EVENT_COMMITMENT_RELEASED: &str = "commitment.released";
/// 违约收束：承诺未履行（载荷带 `why`——违约必须可观测）。落在 `commitment × closed`。
pub const EVENT_COMMITMENT_BROKEN: &str = "commitment.broken";
/// 对等声明：把承诺状态对等宣告给协作方（不是新通道，是普通事件）。
/// 落在 `commitment × asserted`。
pub const EVENT_COMMITMENT_ASSERTED: &str = "commitment.asserted";

// ── 任务格子（S7 第 16–18 步，[roadmap/S03 §3](../../../../docs/stage/S03-多步任务与返工.md)）──
//
// 任务图（DAG）= 事件的 `depends_on` 载荷字段——**数据，不是机制**；
// 返工 = 新增一条事件（`task.rework_created`），不是修改历史（append-only
// 已经提供了撤销语义）。落在 `task × opened / progressed / held / asserted`。
//
// 写方 = `plugins/session/v2_tasks`（会话任务清单 `todo_write` 的转写，
// 04 §3.1 批⑨）；`task.held` 的写方是插话抢占（批⑩）。

/// 开任务：载荷 `{ task_id, depends_on, goal }`。落在 `task × opened`。
pub const EVENT_TASK_OPENED: &str = "task.opened";
/// 推进：执行中的普通事实。落在 `task × progressed`。
pub const EVENT_TASK_PROGRESS: &str = "task.progress";
/// 暂挂（插话抢占 / 等外部资源；[roadmap/S07](../../../../docs/stage/S07-插话与实时打断.md)
/// 里这一格的事件名叫 `task.blocked`，本仓库以 `task.held` 单点定义）。
/// 落在 `task × held`。
pub const EVENT_TASK_HELD: &str = "task.held";
/// 终态：验证通过（验收通过才终态）。落在 `task × asserted`。
pub const EVENT_TASK_ASSERTED: &str = "task.asserted";
/// 返工：判定不合格 ⇒ **新增一条事件**（重开一个返工节点），不是回滚。
/// 落在 `task × asserted`（返工本身是一次质量判定的事实）。
pub const EVENT_TASK_REWORK_CREATED: &str = "task.rework_created";

// ── 控制格子（S8 第 19–20 步，[roadmap/S07 §3](../../../../docs/stage/S07-插话与实时打断.md)、
//    [roadmap/S09 §3](../../../../docs/stage/S09-外部执行与熔断.md)）────
//
// 打断信号与熔断**共用一格**：`control/opened`（kinds 都是 `task.controlled`，
// 以载荷 `reason` 区分）——插话的"停/继续/改道"与外部的"熔断"是同一类事实：
// 一个更快的判定者决定了对在跑事务的处置。不加新机制，只加一格。

/// 控制判定产出（打断处置 / 熔断）。载荷 `{ task_id?, reason, ... }`。
/// 落在 `control × opened`。**抢占判定者只持 JudgeIntent**——无 `reply.*` 写权，
/// 判定者不得直接发言（S07 §5）。
pub const EVENT_CONTROL_OPENED: &str = "task.controlled";

// ── 系统与「欲」格子（S9 第 21 步，[roadmap/S12 §3](../../../../docs/stage/S12-自主层与长期目标.md)）──
//
// 自主层**不是新架构层**：定时触发 = 触发器产出事件（不是旁路）；长目标 = 一条
// `task.opened`，`budget_ms = 86400000`（四层时延的第四个取值，同一参数）；
// 「欲」= `event.entity = conation` 的新取值（02 §2.3 的 E1）。

/// 定时触发：没有用户消息时的自主行为起点。落在 `system × opened`。
/// **触发器产出事件，不是旁路**——自主行为同样走 I1 单通道、I2 带溯源。
pub const EVENT_SYSTEM_TRIGGERED: &str = "system.triggered";
/// 健康自检。落在 `system × progressed`。
pub const EVENT_SYSTEM_HEALTH: &str = "system.health";
/// 「欲」的表达：一条意图出现（E1：欲是数据，走 I1 单通道、I2 带溯源——
/// 无 `produced_by` 的欲事件**构造不出**候选意图，见 ② IntentGate）。
/// 落在 `conation × opened`。
pub const EVENT_CONATION_EXPRESSED: &str = "conation.expressed";

/// 事件实体 —— 语法网格的**行**，闭集（F5，11 个）。
///
/// 新增实体是 L-schema 变化（[plan/03 §2](../../../../docs/plan/03-演进与验证.md)）：
/// 加枚举臂 + ADR 登记，不是机制变更。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Entity {
    Turn,
    Task,
    Artifact,
    Verdict,
    Memory,
    Commitment,
    Conation,
    Thread,
    Control,
    System,
    /// 消息列表上的一次**变更**（编辑 / 删除 / 压缩快照）。见 [`EVENT_MESSAGE_EDITED`]。
    ///
    /// 与 `Turn` 的分界：`turn` 记的是「这一轮说了什么」，本实体记的是
    /// 「列表里的某条被改成 / 去掉了 / 整段被摘要取代了」——它跨轮、不开始任何一轮。
    Message,
}

/// 事件动词 —— 语法网格的**列**，闭集（F5，5 个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Verb {
    Opened,
    Progressed,
    Held,
    Closed,
    Asserted,
}

/// 事件信封：一 条已发生事实的全部元数据。
///
/// **载荷与元数据分两层**：`payload` 是格子自己的数据（如发言内容）；
/// 其余字段是**所有事件共有**的溯源 / 预算元数据（I2 无溯源不声明、I3 到点必答）。
///
/// `seq` 入库前为 `None`——它由 [`crate::symbio_core::store::Store::append`]
/// 在唯一写入口内**分配**，调用方无权指定（append-only + 单调的构造侧保证）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 幂等键：重复 `append` 同一 `event_id` ⇒ [`crate::symbio_core::store::AppendError::Duplicate`]。
    pub event_id: String,
    /// 网格名（数据），如 `"chat.assistant.final"`；落在 `entity × verb` 格子上。
    pub kind: String,
    pub entity: Entity,
    pub verb: Verb,
    /// 所属回合。N3（每 turn 至多一条 final）按它分桶。
    pub turn: u64,
    /// 产生者（Principal，数据：字符串 ID）。
    pub actor: String,
    /// 溯源：产生本事件的上游事件 seq（I2——断言类事件必须非空）。
    pub produced_by: Option<u64>,
    /// 产生本事件花费的毫秒数（I3 记账）。
    pub cost_ms: u64,
    /// 入库时间（Unix 毫秒）。
    pub ts: Timestamp,
    /// 格子自己的数据。
    pub payload: serde_json::Value,
    /// 由 `Store` 分配的全局单调序号；入库前 `None`。
    pub seq: Option<Seq>,
}

impl Event {
    /// 造一条**未入库**事件（seq 待 `Store` 分配）。
    pub fn pending(
        event_id: impl Into<String>,
        kind: impl Into<String>,
        entity: Entity,
        verb: Verb,
        turn: u64,
        actor: impl Into<String>,
    ) -> Self {
        Event {
            event_id: event_id.into(),
            kind: kind.into(),
            entity,
            verb,
            turn,
            actor: actor.into(),
            produced_by: None,
            cost_ms: 0,
            ts: 0,
            payload: serde_json::Value::Null,
            seq: None,
        }
    }

    /// 设置溯源（I2）——链式写法，方便构造。
    pub fn with_produced_by(mut self, seq: u64) -> Self {
        self.produced_by = Some(seq);
        self
    }

    /// 设置预算记账（I3）。
    pub fn with_cost_ms(mut self, cost_ms: u64) -> Self {
        self.cost_ms = cost_ms;
        self
    }

    /// 设置时间戳。
    pub fn with_ts(mut self, ts: Timestamp) -> Self {
        self.ts = ts;
        self
    }

    /// 设置载荷。
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }
}

// ── 载荷里的节点身份（读侧共享）────────────────────────────────────────────
//
// 「一条事件承载哪个消息节点」这件事，**写侧与读侧必须给出同一个答案**：写侧落格
// 时把节点 id 记进载荷，读侧（`rebuild` 折节点树 / `transcript` 折对话行）按同一
// 规则取出来。三个消费方各写一遍的后果是**静默**的——`message.edited` 定位不到它
// 要改的那条节点，表现为「编辑没生效」，而没有任何东西会红。
//
// 规则本体因此住在这里（core 的中性契约层），而不是住在任一读侧实现里。它们只读
// 载荷、不碰任何状态，是纯函数。

/// 载荷里的节点身份：`payload[key]` 优先；缺失（该格不记身份）⇒ 按事件 id 稳定合成。
///
/// ## 「稳定」是硬要求
///
/// 合成 id 用**事件 id**（写方按「一件事一个键」给出，如 `s-{turn}-{n}`）——
/// 同一条事件每次读都得同一个值。若用随机值，每次重开会话都会多出一批「新」节点
/// （前端按 id 合并，id 一变同一句话就是两个身份）。
pub fn event_payload_node_id(e: &Event, key: &str) -> String {
    payload_str(e, key).unwrap_or_else(|| e.event_id.clone())
}

/// 收束 / 进展 / 兜底格的**正文子节点** id。
///
/// ## 为什么不是「读 `text_id` 就完了」
///
/// 落库侧（`message_build::llm_build_assistant_messages`）有一条 **reasoning-only**
/// 规则：思考与正文是同一段内容且没有独立文本回复时，那段内容归 **Text 子节点**、
/// 并**复用 reasoning 的流式 id**（否则同一段内容在存储层出现两份）。重建若只认
/// `text_id`，reasoning-only 那一轮会得到 `{turn_id}-text` 这个**落库侧并不存在**的
/// id——于是那条正文在实时面与重建面是两个身份，且 `message.edited` 永远定位不上它。
///
/// 判据与落库侧**逐字相同**（同一段文本比较），因为它就是同一个问题的答案。
pub fn event_produced_text_node_id(e: &Event) -> String {
    let text = payload_str(e, "text").unwrap_or_default();
    let reasoning = payload_str(e, "reasoning").unwrap_or_default();
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();
    let id = if reasoning_only {
        payload_str(e, "reasoning_id").or_else(|| payload_str(e, "text_id"))
    } else {
        payload_str(e, "text_id")
    };
    id.unwrap_or_else(|| {
        // 载荷连 `turn_id` 都没有（答话轮 / 反射档：没有流式节点）⇒ 与落库侧同一条
        // 回退：正文子节点的名字由 Turn 根派生。
        format!("{}-text", payload_str(e, "turn_id").unwrap_or_default())
    })
}

/// 工具**结果**节点的 id（`artifact.added` 载荷 `result_id`）。
///
/// 恢复路记 `null`（那一轮没有流式结果节点）⇒ 按**事件 id** 合成。合成用的**不是**
/// `tool_call_id`：结果与调用是两张卡，共用 id 会让前端把它们合成一条。
pub fn event_tool_result_node_id(e: &Event) -> String {
    payload_str(e, "result_id").unwrap_or_else(|| format!("{}-result", e.event_id))
}

/// 一条事件在**读侧**承载的节点身份（`None` = 该格不产节点）。
///
/// ## 它是「哪一格对应哪个节点」的单点答案
///
/// 三个消费方都按它认节点：`rebuild`（节点树）、`transcript`（对话行）、
/// `prompt_fidelity`（变更索引的配对）。各写一遍的后果是**静默**的——
/// `message.edited` 定位不上要改的那条节点，表现为「编辑没生效」。
///
/// ## 只给「内容节点」，不给全部节点
///
/// 收束格一次承载 `[Turn 根, Reasoning 子, Text 子, rounds…]` 一串节点，这里只回
/// **正文子节点**（对话里那一句就是它）。对话读侧（转写行 / prompt 复核）只需要它；
/// 节点树（`rebuild`）要的是全部，所以它自己按节点逐个取。
pub fn event_node_of(e: &Event) -> Option<String> {
    match (e.entity, e.verb, e.kind.as_str()) {
        (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => Some(event_payload_node_id(e, "id")),
        (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => {
            Some(event_payload_node_id(e, "id"))
        }
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED) => {
            Some(event_payload_node_id(e, "id"))
        }
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_OPENING) => {
            Some(event_payload_node_id(e, "id"))
        }
        (
            Entity::Turn,
            Verb::Closed | Verb::Progressed,
            EVENT_ASSISTANT_FINAL | EVENT_ASSISTANT_FALLBACK | EVENT_ASSISTANT_PROGRESS,
        ) => Some(event_produced_text_node_id(e)),
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => {
            Some(event_tool_result_node_id(e))
        }
        _ => None,
    }
}

/// 取载荷里的**非空**字符串字段（空串读出来是「这个字段没记」，不是「记了空」）。
fn payload_str(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 信封的最小契约（[plan/01 §2](../../../../docs/plan/01-核心架构.md) 的 `type Event: EventEnvelope`）。
///
/// `Store` 只需要知道三件事：幂等键是什么、seq 在不在、seq 怎么赋值。
/// 剩余字段对存储层不可见——将来 WAL / 分片实现携带更富的信封也不动 trait（F1）。
///
/// `Send + Sync` 是事件作为**纯数据**的本性（快照要跨线程交给投影），
/// 放在超 trait 上，任何实现都无法造出不能跨线程的事件。
pub trait EventEnvelope: Clone + Send + Sync {
    /// 幂等键。
    fn event_id(&self) -> &str;
    /// 入库前 `None`。
    fn seq(&self) -> Option<Seq>;
    /// **仅由 `Store` 在唯一写入口内调用。**
    fn assign_seq(&mut self, seq: Seq);
}

impl EventEnvelope for Event {
    fn event_id(&self) -> &str {
        &self.event_id
    }

    fn seq(&self) -> Option<Seq> {
        self.seq
    }

    fn assign_seq(&mut self, seq: Seq) {
        self.seq = Some(seq);
    }
}
