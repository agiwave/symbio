//! 事件契约 —— v2 事件溯源的共享类型层（[plan/01 §2 / §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 为什么是独立模块
//!
//! 施工视图的六个包（`actors` / `projections` / `store` / `adapters` / `governance`）
//! 彼此**禁止互相依赖**，唯一例外是「事件与 View 的类型定义」——那是共享契约
//! （[plan/05 §3.1](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")，01 §13.5「契约从边上挪到中心」）。
//! 因此事件类型必须住在**中性层**：`store`（④）与 `projection`（③）都只 import 本模块，
//! 谁也拿不到谁的句柄。
//!
//! ## 冻结锚点（[plan/03 §1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）
//!
//! - **F5**：动词 5 个、能力 7 个、模式 3 个 —— 本模块冻结动词与实体的**闭集**
//!   （`Verb` / `Entity` 枚举）。新增格子 = `match` 加一臂，是数据变化不是机制变化；
//!   枚举而不是字符串，是为了让"网格之外的事件"**编译不过**（J3）。
//!   注意 J3 钉住的是**行与列**这两维：字符串确实写不出第 12 个实体。
//! - 事件**语法网格**（01 §6）：`实体 × 动词` 的格子上有若干**名字**。名字是数据，
//!   加名字不加枚举。名字 ↔ 格位这条对应关系的**主人是 owner 的 `EventDecl`**
//!   （DE-024 D-197 / D-203）：内核只留自己的协议五格，其余每一条名字随声明它的
//!   插件走。判据不在编译期（名字是字符串），而在**唯一写入口**——装配表不认识的
//!   名字、与自述格位不一致的名字，都落不进事实源（`AppendError::OffGrid`）。
//!
//!   迁移期的内核名字表（`event::grid::CELLS`）已随 DE-024 D-203 归零批退役：
//!   21 条业务名字全部由 owner 声明，`kernelEvents` / `cellsRows` 两格降到 0。

use serde::{Deserialize, Serialize};

/// 网格上的一个格子：实体（行）× 动词（列）。
///
/// 它此前是 `event::grid::CELLS` 那张「名字 → 格位」表的一部分；表随 DE-024 D-203
/// 归零批退役（业务名字全部由 owner 的 `EventDecl` 声明），`Cell` 作为**格位本身**
/// 的类型留下来——`EventDecl.cell` / `Frozen::event_cell` / `AppendError::OffGrid`
/// 都用它。
///
/// 表退役之后「这个名字落在哪一格」只剩两个来源：装配表（`Frozen`，owner 交来的
/// 声明）与各插件清单生成的共享表（`symbio_plugin_api::events::SHARED_EVENTS`）。
/// 内核不再认识任何业务名字，也就不再需要一张自己的名字表。
pub type Cell = (Entity, Verb);

/// 一条事实在**守卫判据**里扮演的角色（DE-024 D-203 裁决二）。
///
/// 这四档答的是「core 的判据为什么要看这条事实」，不是「这条事实叫什么」：
/// 名字是数据、可以很多；语义只有四档，加一档是架构变更（必须论证）。判据经
/// `Frozen::holds_role` 读它，不再拿名字比字符串——于是「新增一条共享事件名」
/// 与「改动架构」脱钩（DE-024 D-203）。
///
/// **每档都必须有人居住**：装配口逐档查居住（`Frozen::unmanned_roles` ⇒
/// `Gap::UnmannedRole`），门禁 CS-008 判整仓。少了这条，owner 删掉角色会让对应判据
/// **静默不判**——判据从「拒绝」变成「看不见」，正是 J3 点名的失效形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactRole {
    /// 本轮的**定稿发言**（同格还有兜底格，它不算定稿：`final_unique_per_turn` 只拦这一档）。
    FinalizesTurn,
    /// 回答「这一轮为什么停」的否定事实（收束格之外唯一的一档）。
    StopsTurn,
    /// 本轮预算档的声明来源（C5 的读数从这里取）。
    DeclaresBudget,
    /// 同一任务的**重做轮**（`rework_bounded` 的计数对象——终止前提）。
    CountsRework,
}

impl FactRole {
    /// 缺口报告里点名用的稳定拼写（与枚举变体一一对应，不参与判定）。
    pub const fn as_str(self) -> &'static str {
        match self {
            FactRole::FinalizesTurn => "finalizes_turn",
            FactRole::StopsTurn => "stops_turn",
            FactRole::DeclaresBudget => "declares_budget",
            FactRole::CountsRework => "counts_rework",
        }
    }

    /// 闭集全集：`seal` 逐档查居住，新加一档自动进检查面。
    pub const ALL: &'static [FactRole] = &[
        FactRole::FinalizesTurn,
        FactRole::StopsTurn,
        FactRole::DeclaresBudget,
        FactRole::CountsRework,
    ];
}

/// 名字 ↔ 格位 ↔ 角色：写闸与 core 判据共用的词汇面。
///
/// 判据拿到的是**这张表**（`&[EventDecl]`），不是自己抄的一份名字。
pub fn holds_role(vocab: &[EventDecl], kind: &str, role: FactRole) -> bool {
    vocab
        .iter()
        .find(|e| e.name == kind)
        .is_some_and(|e| e.roles.contains(&role))
}

/// 一个**事件名**的声明：名字、它在网格上的格位，以及它在守卫判据里扮演的角色。
///
/// 这是事件词汇的登记口（DE-024 D-197）：`Entity` × `Verb` 是封闭语法，**名字不是**。
/// 加一个事件名 = 在 owner 插件里加一行 `EventDecl`，内核不认识这个名字。
///
/// ## 为什么住在 `event` 域而不是 `decl`
///
/// 它声明的是「这个名字落在哪一格、扮演什么判据角色」——回答的是事件词汇与网格的对应，
/// 不是载荷形状。`decl` 域（`FactDecl` / `ViewDecl` / `ActorDecl`）答的是另一组问题：字段、
/// 读法、执行体收口。判据 7 的输入是 `FactDecl`，网格的输入是本类型，两者合并会逼其中
/// 一方说谎（边界分析见 [`crate::decl`]）。
///
/// 两表同名不冲突：一条事实若要进环，同时交 `EventDecl` 与 `FactDecl` 即可；`seal`
/// 只要求**插件声明的事实**都能在事件表上找到同名行（协议五格由内核预置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDecl {
    /// 事件名（线上 `kind` 的字面值）。
    pub name: &'static str,
    /// 名字落在哪一格。写闸按它拒绝「自述挪了一格」的事件。
    pub cell: Cell,
    /// 名字扮演的守卫角色。绝大多数事件不扮演任何角色（`&[]`）。
    pub roles: &'static [FactRole],
}

impl EventDecl {
    /// 只登记格位（不扮演任何判据角色）——绝大多数事件走这个构造器。
    pub const fn new(name: &'static str, cell: Cell) -> Self {
        EventDecl {
            name,
            cell,
            roles: &[],
        }
    }

    /// 同时登记格位与角色：core 的守卫判据要靠这一行找到它。
    pub const fn with_roles(name: &'static str, cell: Cell, roles: &'static [FactRole]) -> Self {
        EventDecl { name, cell, roles }
    }
}

/// 不可伪造的单调序号（[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 字段私有：**只有 `Store` 能构造**（`pub` 在 kernel crate 内，等价于拆分前的
/// `pub`），调用方只能拿到与比较——自己 `Seq(5)` 伪造一个序号编译不过。
/// 这就是「类型强制单调」的构造侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Seq(u64);

impl Seq {
    /// 仅事件源实现可构造（kernel crate 内；对 `symbio_core` 是同一份共享语法）。
    pub fn new(v: u64) -> Self {
        Seq(v)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

/// 事件时间戳：Unix 毫秒（UTC），口径同 `clock_now_ms`。
pub type Timestamp = i64;

// ── 事件名字表（只剩协议五格）────
//
// 名字是**数据**（加名字不加枚举，01 §6）；但名字必须**单点定义**——散在调用点的
// 字符串字面量必然漂移。
//
// **归属（DE-024 D-197 / D-203）**：这里只留内核**自己**持有的名字——协议五格与两个
// 载荷键常量。业务名字一条都不在这里：每条名字随声明它的 owner 走，格位同批跟着
// `EventDecl`，跨插件的名字经清单 `shared_events` 投影（`symbio_plugin_api::events`）。
//
// 「加一个能力要不要改这个文件」因此有了可判的答案：**不要**。新名字只出现在 owner
// 的 `EventDecl` 与它的清单里，CS-005 的 `kernelEvents` 不为此增行。

// ── 协议族（port protocol）：**对外调用落成事实**──
//
// 这五个名字是**新增**，不是替换：现网的事实名全是业务词汇（`memory.recalled` /
// `task.rework_created` / `chat.assistant.final` / `commitment.broken` …），
// 有各自真实的读方，**一个不动**。把它们压成几个协议名不是简化，是销毁语义。
//
// 补上的是**一类从来不存在的事实**：今天「我要调模型」「我要调工具」只活在内存里
// （DE-014 §2.1 的「意图」）。落成事实之后，每一次对外调用都可回放、可恢复、可审计。
//
// **端口是载荷，不是事实种类**：只有这五个名字带 `port` 字段（DE-015 §2.4 的裁决），
// 端口的取值（`turn` / `model` / `tool` / …）写在载荷里。**加一个端口 = 0 个新事实名。**
//
// 落格：**五个都在 `turn × asserted`**，逐条理由写在各自的文档里。

/// **该调了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 决策者——写下「这一轮要发起这次调用」的那一方。
///
/// ## 为什么落 `turn × asserted`
/// 它是一条**关于这一轮的断言**（「这一轮欠一次 `port` 调用」），不是状态迁移：
/// `turn × opened` 是轮次的**开始**（一个 turn 只能有一次），`turn × closed` 是轮次的
/// **结束**（收束格），`turn × progressed` 是**用户看得见**的动静（`chat.assistant.progress`）。
/// 协议事实是**机器对机器**的账：不进对话面、不产节点、不进请求包。
///
/// ## 载荷
/// `port` = 端口名；`id` = **逻辑 id**（同一笔调用的各次尝试共用它，幂等键按它去重）；
/// `attempt` = 尝试号（重试时 +1）。
pub const EVENT_REQUEST: &str = "request";

/// **已经派出去了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// **引擎**（`before` 标记位），不是搬运者——搬运者的 `run` 可能不产生事实就返回。
/// 若由它写 `sent`，「派出去了」就依赖对方自觉：它一忘，`dispatch` 行永远不还清，
/// 而**没有任何东西会红**（DE-015 §2.3； 判据 2）。
///
/// ## 为什么落 `turn × asserted`
/// 与 `request` 同格同族：一次调用的两个阶段是同一条账的两个断言。
/// 「已派出」不是用户看得见的动静，故不占 `turn × progressed`。
pub const EVENT_SENT: &str = "sent";

/// **回来了**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 搬运者（拿到应答的一方）。
///
/// ## 为什么落 `turn × asserted`
/// 「有应答」是一条关于这一轮的断言。它**不是**收束格：轮子由
/// `chat.assistant.final` / `fallback` 收束，一次端口调用的应答不结束一轮。
pub const EVENT_RESPONSE: &str = "response";

/// **没成功**（载荷 `{ port, id, why }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 搬运者；引擎在执行体 `run` 返 `Err` 时兜底写它——**失败也是事实**，
/// 义务环里没有异常分支（DE-015 §2.6 契约 3）。
///
/// ## 为什么落 `turn × asserted`（而不是 `closed`）
/// 与 `authz.denied` 同一条纪律：一次端口调用失败**不关轮**——轮还可能重试、
/// 走降级、或以兜底话术收束。把它算成收束会让 `unresolved_turns` 少报一轮
/// 「静默中断」，而那正是**最该被发现的失效**。
///
/// ## 载荷
/// 没有 `attempt`：`failed` 关的是**逻辑 id**（所有尝试），不是「某一次尝试失败」。
pub const EVENT_FAILED: &str = "failed";

/// **这次尝试作废，换一次**（载荷 `{ port, id, attempt }`）。落在 `turn × asserted`。
///
/// ## 谁写
/// 租约看护者（`lease_keeper`）——`sent` 之后长时间无应答时，它按
/// `PortDecl.max_attempts` 决定重试（落 `retry` + 新一轮 `request`）或判死
/// （落 `failed`）。
///
/// ## 为什么需要它
/// 没有它，`sent` 之后崩溃 ⇒ `await` 行永远 blocked ⇒ 那笔账永远挂着，
/// 而日志上「发出去了」与「卡住了」长得一样（DE-015 §2.6 契约 7）。
pub const EVENT_RETRY: &str = "retry";

// ── 授权（authz）：**拒绝也是一条事实**──

// ── 停机原因：**一个字段、一个闭集**──

/// 停机原因——「这一轮为什么停」的唯一答案，取值是**闭集**。
///
/// ## 它收掉的是三个形状
///
/// 此前同一个问题有三个载体，语义互相重叠且都不够用：
///
/// | 旧形状 | 问题 |
/// |---|---|
/// | `payload.status` ∈ `completed` / `failed` / `aborted` | 结果与原因混在一个字段里 |
/// | `payload.terminated.reason` | 恒为 `"aborted"`，等于没有信息 |
/// | `payload.why` | 自由文本，不可判 |
///
/// 于是「这一轮为什么停」只能靠读三个键 + 猜。而它正是重启后最需要回答的问题：
/// 义务环里没有 `Halt` 这个值——"停"只有一种表达，就是"没有债了"，那么
/// "为什么没有债了"必须写在事实里（DE-014 需要一个运行期分支
/// `None => halt(NoRule)`；义务环里那个分支不存在——装载期"每行收口"已拦住，
/// 所以闭集里**没有 `no_rule`**，这不是遗漏）。
///
/// ## 与 `status` 的分工
///
/// `status` 留下，但只留**结果面**三值（`completed` / `failed` / `aborted`）：
/// 结果与原因是两个维度，合成一个字段会让「截断但答出来了」「失败且被拒」
/// 这类组合表达不出来。
///
/// ## `detail`：人话细节，唯一不可判的字段
///
/// 闭集回答"是哪一类"，`detail` 回答"具体是什么"（如适配器原始错误文本）。
/// 二者分开是因为：把细节塞进闭集会让它不可判，把闭集市写成自由文本会让它
/// 不可统计。**只许对 `reason` 做分支，`detail` 只给人读。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureReason {
    /// 说完了（自然结束）。
    Completed,
    /// 用户打断（或首请求前到达的中止）。
    Interrupted,
    /// 上界耗尽——`finish_reason = length`：步数 / 预算 / max_tokens 掐断。
    Exhausted,
    /// 主路径失败，兜底顶上。
    Fallback,
    /// 收束闸拒绝。**不产生收束格**——被拒的一轮仍然未收束，
    /// 这个值只落在 `authz.denied` 上。
    Denied,
}

impl ClosureReason {
    /// 全部取值——审计判据的闭集由这里派生，**不另写一份**。
    pub const ALL: [ClosureReason; 5] = [
        ClosureReason::Completed,
        ClosureReason::Interrupted,
        ClosureReason::Exhausted,
        ClosureReason::Fallback,
        ClosureReason::Denied,
    ];

    /// 线上拼写（载荷 `reason` 的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            ClosureReason::Completed => "completed",
            ClosureReason::Interrupted => "interrupted",
            ClosureReason::Exhausted => "exhausted",
            ClosureReason::Fallback => "fallback",
            ClosureReason::Denied => "denied",
        }
    }

    /// 从载荷字符串解析（读侧 / 审计用）。`None` = 越界——这正是判据要抓的。
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == raw)
    }
}

// ── 产物格子（S02 §3，[roadmap/002-工具调用与产物.md](../../../../docs/003-design/032-工具调用与产物.md "DE-032-工具调用与产物")）──
//
// 工具**结果**是产物事实：模型请求了工具、工具产出了这段东西。工具调用**本身**
// 不单独占格（同一件事两格必然漂移）；`produced_by` 指向**本轮用户格**，即
// 「这个产物是本轮用户请求的后果」——S02 §3 的验收断言正是 `caused_by` 指向发起
// 它的 `task/turn`。

// ── 消息列表变更的格子（D-053 / D-054）──
//
// 「消息列表被改过」也是**发生过什么**。在轨 S 之前，编辑 / 截断只重写
// `messages.json`，事实源里一个字都没有 ⇒ 主读路径一旦切到「从 WAL 重建」，
// **被截断的消息会复活、编辑过的内容会回退**。
//
// 这一族是**闭集**（D-054 决策 1）——列表的改动只有四种，三个节点级 + 一个列表级：
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

// ── 记忆格子（S5，[roadmap/S06 §3](../../../../docs/003-design/036-长期记忆与语义检索.md "DE-036-长期记忆与语义检索")）────
//
// 校准断言 `memory.calibrated`（`memory × asserted`）不在这里：写方只有 `plugins/loop`
// 一家，名字与格位由它自己的 `EventDecl` 交出（`control_decl`），读侧投影的名字由调用方
// 在构造期交进 core（DE-024 D-205 / D-204）。
//
// 为什么它不能继续复用 `memory.recalled`：复用是 S11 §3 的省格决定，代价落在**读侧**——
// 校准投影只能靠载荷有没有 `skill_id` + `fallback` 认自己的输入，同一张格子两种语义，
// 判别退化成「谁恰好带了这两个字段」，那不是闭集：任何将来带这两个字段的事件都会被算进
// 置信度。给它一格之后读侧按名字过滤，判别回到闭集；写侧不变（仍是轮末 `derived_facts`
// 收束族，**不开第二写通道**——I1）。这段理由是**这一格**的，随名字住在 owner 声明旁。

// ── 承诺格子（S6 第 15 步，[roadmap/S08 §3](../../../../docs/003-design/038-多主体与对等承诺.md "DE-038-多主体与对等承诺")）──
//
// 立约 = `commitment` 实体的一格事件（**加格子，不加机制**——对等协作是图，
// 子智能体递归的树表达不了，才需要这组格子）。声誉是它的投影（`projection = reputation`）。

// ── 任务格子（S7 第 16–18 步，[roadmap/S03 §3](../../../../docs/003-design/033-多步任务与返工.md "DE-033-多步任务与返工")）──
//
// 这一节现在是空的。五格（开任务 / 推进 / 暂挂 / 终态 / 发放）与 `task.ready` /
// `task.rework_created` 同属一个 owner：写方与调度者都在 `plugins/loop`，`session` 只作为
// **第二个消费方**读它们——名字因此登记进 owner 清单（`shared_events`，判据 CS-007），
// 格子与租约纪律随 owner 的 `EventDecl` 走（DE-024 D-204 / D-206）。
//
// 领域形状留在 roadmap，不在语法里：任务图是事件的 `depends_on` 载荷字段（**数据不是
// 机制**，C14 判无环）；返工 = 新增一条事件而不是修改历史（append-only 已提供撤销语义）。

// ── 控制格子（`task.controlled`）──
//
// 名字不在这里：写方在两个插件各一个（session 的打断处置、loop 的熔断与拒因），
// 它因此是**跨插件事件名**，登记口是 owner 清单的 `shared_events:` 行
// （`symbio_plugin_api::events`，判据 CS-007）——内核不为任何业务事件命名
// （DE-024 D-203 / D-204）。格子语义与写侧纪律随 owner 的 `EventDecl` 走。

/// 「这是一次自主开轮」在**地址写入体**上的判别键（`ChatNode.meta` 的键名）。
///
/// ## 为什么一个写体的键名住在事件词汇表
///
/// 它不是事件名，而是**同一个事实的另一种写法**：写体 `meta.trigger` 声明的是
/// 「这一轮由系统自己发起」，落进事实源就是 `system.triggered` 那一格。
/// 两侧必须同源——`meta` 是自由 JSON，键名拼错编译器管不到，生产方（调度侧）与
/// 消费方（存储所有者的写臂）各写一份就是两条漂移的口径（[D-077](../../../symbio-plugin-heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 裁决二）。
///
/// 形状：`meta.trigger = { "kind": <EVENT_TRIGGER_KIND_*>, "idle_ms": N }`，
/// **在写臂就地消费**，不要求它活到转写。
pub const EVENT_TRIGGER_META_KEY: &str = "trigger";
/// 触发方式的取值：**定时触发**（心跳一族的调度器）。落在 `system.triggered`
/// 的载荷 `kind` 上，也落在写体 `meta.trigger.kind` 上——同一个名字说同一件事。
pub const EVENT_TRIGGER_KIND_SCHEDULED: &str = "scheduled";

// ── 裁决格子（S13 §3，[roadmap/S13](../../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会")）──
//
// 集体决策 = 「多个主体对同一 verdict 的竞争 + 仲裁者」：`verdict` 实体（F5 闭集
// 里一直有这一行）在 S13 取的第一个值就是**淘汰裁决**——声誉投影低于阈值的主体
// 被收回 `assign.work`（§2「淘汰」行）。**不加新格子**：`verdict × asserted`
// 正是「对一次判定盖章」，与承诺族的 `commitment.asserted` 同形。

/// 事件实体 —— 语法网格的**行**，闭集（F5，11 个）。
///
/// 新增实体是 L-schema 变化（[plan/03 §2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
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
    /// 消息列表上的一次**变更**（编辑 / 删除 / 压缩快照）。名字由 owner 声明
    /// （`plugins/session` 的 `message.edited` 等四条），内核不持有拼写。
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
/// `seq` 入库前为 `None`——它由 [`crate::store::Store::append`]
/// 在唯一写入口内**分配**，调用方无权指定（append-only + 单调的构造侧保证）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// 幂等键：重复 `append` 同一 `event_id` ⇒ [`crate::store::AppendError::Duplicate`]。
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
    event_output_text_node_id(
        payload_str(e, "text").as_deref().unwrap_or_default(),
        payload_str(e, "reasoning").as_deref().unwrap_or_default(),
        payload_str(e, "text_id").as_deref(),
        payload_str(e, "reasoning_id").as_deref(),
        payload_str(e, "turn_id").as_deref(),
    )
}

/// [`event_produced_text_node_id`] 的**字段版**：同一条规则，供写侧在落格前算好后写进
/// `answer_id`（模型调用的事实自带它那一轮的正文身份）。
///
/// 写侧与读侧共用这一份实现：两处各算一遍的后果是静默的——`response` 记的 `answer_id`
/// 与收束格/转写行算出来的值不同，表现就是「编辑定位不上那条正文」。
pub fn event_output_text_node_id(
    text: &str,
    reasoning: &str,
    text_id: Option<&str>,
    reasoning_id: Option<&str>,
    turn_id: Option<&str>,
) -> String {
    let reasoning_only = !reasoning.trim().is_empty() && reasoning.trim() == text.trim();
    let id = if reasoning_only {
        reasoning_id.or(text_id)
    } else {
        text_id
    };
    id.map(str::to_string).unwrap_or_else(|| {
        // 载荷连 `turn_id` 都没有（答话轮 / 反射档：没有流式节点）⇒ 与落库侧同一条
        // 回退：正文子节点的名字由 Turn 根派生。
        format!("{}-text", turn_id.unwrap_or_default())
    })
}

/// 工具**结果**节点的 id（`artifact.added` 载荷 `result_id`）。
///
/// 恢复路记 `null`（那一轮没有流式结果节点）⇒ 按**事件 id** 合成。合成用的**不是**
/// `tool_call_id`：结果与调用是两张卡，共用 id 会让前端把它们合成一条。
pub fn event_tool_result_node_id(e: &Event) -> String {
    payload_str(e, "result_id").unwrap_or_else(|| format!("{}-result", e.event_id))
}

/// 收束 / 进展 / 兜底格的**正文节点** id：载荷 `answer_id` 优先，缺失时按同一轮
/// 载荷派生（[`event_produced_text_node_id`]）。
///
/// ## 为什么这条规则留在内核，而「哪一格算产节点」不在
///
/// 「`answer_id` 缺失时怎么合成」是**纯载荷规则**：它只读这一条事件的字段，不认任何
/// 名字、不需要知道是哪一格的产物。而「哪一格产节点」是**对话折叠的知识**——它随
/// 三张词汇表走（DE-024 D-203 / D-206），core 的 `projection::vocab::slot_node_of`
/// 按槽位给答案。内核在这里只提供两个可复用的载荷取法，不判格子。
pub fn event_answer_node_id(e: &Event) -> String {
    payload_str(e, "answer_id").unwrap_or_else(|| event_produced_text_node_id(e))
}

/// 取载荷里的**非空**字符串字段（空串读出来是「这个字段没记」，不是「记了空」）。
fn payload_str(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 信封的最小契约（[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的 `type Event: EventEnvelope`）。
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
    /// 落格前给「没带时刻」的信封兜底——**仅由 `Store` 在唯一写入口内调用**，
    /// 与 [`EventEnvelope::assign_seq`] 同一条纪律（入库形态由存储层说了算）。
    ///
    /// 为什么必须存在：`d781a6c7` 号会话 548 个事件的 `ts` 全为 0——上游十余处
    /// `Event::pending` 构造点，填了 `timestamp` 的是少数，`0` 便一路漏进事实源。
    /// `0` 不是「未知」而是**谎报**（假装都发生在纪元起点），时序投影因此整体失真。
    /// 逐构造点手改必然再漏（同一件事的手写实现已证明会漂移），故收口在入库处：
    /// 没带时刻的事件以**落格时刻**为准，事实源里不再有 `ts = 0` 的事件。
    /// 默认实现不动——没有时刻概念的信封保持原样。
    fn fallback_timestamp(&mut self, _now_ms: i64) {}
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

    fn fallback_timestamp(&mut self, now_ms: i64) {
        // `0` 是 [`Event::pending`] 的「没填」哨兵，不是事实；带了时刻的不碰。
        if self.ts == 0 {
            self.ts = now_ms;
        }
    }
}
