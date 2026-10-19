//! 本机部署的**授权策略表**（写侧 grants + 读侧可见域，[plan/01 §7](../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。
//!
//! ## 为什么这张表住 `symbio_core`
//!
//! 表里写的是**部署事实**——本机有哪几个主体、各持有什么能力。三方的边界是：
//!
//! | 谁 | 拥有 | 不拥有 |
//! |---|---|---|
//! | 同处 core 的 `governance` | 能力闭集（7 个封顶）、成对性与两条判定的**机制** | 「谁」是部署事实 |
//! | 本模块 | 主体清单与各自的 grants / 可见域 | 判定语义（复用 `governance` 的函数） |
//! | 各插件 | 自己那道闸上「现在该判谁」 | 表本身——插件互不可见（E-009），任一插件持有它都会逼别的插件跨插件引用 |
//!
//! 表是**架构元素**：「谁持有什么」与「能不能」是同一张表的两半，分家的代价是
//! 「表授予 A、闸门判 B」这种错位**没有任何东西会变红**——两边都是合法能力名，
//! 事后没有任何东西能指出是哪半边写错了。故它与机制同住一棵模块树，
//! 整块经 [`pub(crate)`](crate::symbio_core) 开给全 crate（见 `symbio_core/mod.rs`）：
//! 跨插件可见，却不进对外公开面——表里写的是部署事实，不是契约。
//!
//! ## fail-closed 三层
//!
//! 1. **构造期**：认不出的能力名 ⇒ 拒绝整体构造（`PolicyError`，core 定义），
//!    降级为**空矩阵**并告警——不是「跳过那一行」，笔误不能变成一条悄悄生效的拒绝；
//! 2. **写侧**：运行器落格前判（`turn_runner::closure_granted`，按**种类**分两臂）：
//!    发言收束（收束格）判 [`can_reply`](PermissionMatrix::can_reply)，产物收束
//!    （`artifact.added`）判 [`can_produce_artifact`](PermissionMatrix::can_produce_artifact)
//!    （[ADR-067](../../../docs/010-adr/067.md "ADR-067")）。未持有 ⇒ 拒绝入格：发言臂下
//!    该轮留在未收束态——除非它落过产物格（C4 与产物闸同批认可「产物即收束」，
//!    见 `invariants::unresolved_turns`）；
//! 3. **读侧**：`stats` 按读方声明判 [`can_see`](PermissionMatrix::can_see)——属主
//!    是**内容的属主**（`SESSION_OWNER`，部署事实）而非事件作者，于是今天的判读
//!    就是「属主放行、其余整体拒绝」；`thread_private` 缺省（C10）。
//!
//! ## 两张表里的「人」
//!
//! `user.message` **不入能力表**：能力按「智能体的权威动作种类」划分，人的发言
//! 不是动作种类（落不进去就走 [03 §3](../../../docs/004-plan/003-演进与验证.md "PL-003-演进与验证") 的 ADR）。
//! 读侧因此也不靠矩阵成员身份，而靠 **owner 相等**：`thread_private` 下属主是
//! 本机会话的属主，语义见 `governance::can_see`。
//!
//! ## 本表会怎么长
//!
//! 多主体加固（[plan/11 批 2](../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") /
//! 04 §3.1 批⑧）落地时，子智能体各成一行——行的形状不变，仍是
//! `(主体, 能力名, 可见域)`。
//!
//! 自主发起者那一行已按 [S12 §3](../../../docs/009-task/132-自主层与长期目标.md "T-132-自主层与长期目标")
//! 先落地：它是本表里**第一行「比主智能体窄」**的记录（只持 `define.work`）。这行同时
//! 说明一件事——**「同角色」是平凡值而不是规则**：派生只服务**未登记**的主体，
//! 一旦某个主体被显式登记，那一行就是它的权威能力集。
//!
//! 抢占判定者那一行（`session/reflex`，只持 `judge.intent`）按 [S07 §5](../../../docs/009-task/127-插话与实时打断.md "T-127-插话与实时打断")
//! 随后落地，与自主行同形：**正文写着「不得直接发言」而表里没有行**时，`matrix_for`
//! 的派生分支会把主智能体整套（含 `reply.*`）给它——断言于是被静默推翻。

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::symbio_core::governance::PermissionMatrix;
use crate::symbio_core::{is_agent_id, principal_namespace, VisScope};

/// 本机主智能体的身份——**表里那一行的名字**与收束事件的 `actor`
/// 是同一个常量：授权判定的对象若与实际写入的主体各写一份字符串，两边迟早漂移
/// 到「判的是 A、写的是 B」而没有任何东西会变红。
///
/// id 是**可路由地址**（[ADR-069 §决策 2](../../../docs/010-adr/069.md "ADR-069")）：
/// `<namespace>/<name>`——`session` 是本机智能体的归属 namespace。
pub(crate) const PRINCIPAL_MAIN: &str = "session/main";

/// 本机用户的身份——用户发言事件的 `actor`。
///
/// [ADR-069] 把它从裸字面量 `user` 归一为可路由地址 `person/local`——
/// `person` 是概念 namespace（无插件归属时用概念名）。语义不变：`user.message`
/// **不入能力表**（能力按「智能体的权威动作种类」划分，人的发言不是动作种类），
/// 本表**不为它立行**——`matrix_for` 走「非 agent 命名空间直接用生产表」的分支，
/// 表里没有 ⇒ `can_reply` fail-closed。
pub(crate) const PRINCIPAL_USER: &str = "person/local";

/// **vdfs 写工具**的身份（[ADR-069 §决策 2]）——工具响应叶子的 `principal`；
/// 与调用它的 `session/<caller>` 平级（工具请求节点仍归调用者）。id 的 namespace 是插件名。
///
/// 本表给它一行：`produce.artifact`——工具的权威动作是**产生产物**
/// （`artifact.added` 的响应正文），与 `session/worker` / `session/peer-b` 同款独持。
/// 今天写侧闸按会话主体判，不按工具主体判，故这一行是**声明性**的——它把「工具
/// 是主体」写进表；[T-147] 之后 `plugin::attributed` 会按 kind=Tool 补上这个
/// principal 值，判据不变。
pub(crate) const PRINCIPAL_TOOL_VDFS_WRITE: &str = "vdfs/write";

/// **网页取回工具**的身份。
pub(crate) const PRINCIPAL_TOOL_WEB_FETCH: &str = "web/fetch";

/// **本地检索工具**的身份。
pub(crate) const PRINCIPAL_TOOL_CONTENT_SEARCH: &str = "local/content_search";

/// **压缩器**系统主体（[ADR-069 §决策 2]）——compaction 事件的将来发言方。
///
/// 这里**不立 Rust 常量**：今天压缩节点由会话主体落、写侧没有消费方，`dead_code`
/// 会逐条点名，而 R-002 的棘轮不许为「将来会用」加承认标记——常量的唯一 owner 是
/// [ADR-069 §决策 2] 那张表，写侧接线时（[T-147] 把「压缩由谁写」的判据换掉）随接线
/// 一批立常量、随布线摘标记，两步同批发生。
///
/// 自主发起者的身份（[roadmap/S12 §3](../../../docs/009-task/132-自主层与长期目标.md "T-132-自主层与长期目标")）
/// ——表里那一行的名字与 `AutonomousInitiator` 四个事件的 `actor` 是同一个常量。
///
/// 与 [`PRINCIPAL_MAIN`] 同理：判定用的主体名与实际写入的主体名各写一份字符串，两边
/// 迟早漂移到「判的是 A、写的是 B」而没有任何东西会变红——**这里尤其致命**，因为这一行
/// 存在的全部理由就是**少给**（它不持 `reply.*`）；名字对不上，那行就成了一条谁也管不到的
/// 死记录，而自主行为照旧能冒充用户对话。
pub(crate) const PRINCIPAL_AUTONOMOUS: &str = "session/autonomous";

/// 抢占判定者的身份（[roadmap/S07 §3](../../../docs/009-task/127-插话与实时打断.md "T-127-插话与实时打断")）
/// ——表里那一行的名字与 `PreemptionDecider` 三个事件（`task.held` /
/// `task.progressed` / `control.opened`）的 `actor` 是同一个常量。
///
/// 与 [`PRINCIPAL_AUTONOMOUS`] 同理：判定用的主体名与实际写入的主体名各写一份字符串，
/// 两边迟早漂移到「判的是 A、写的是 B」而没有任何东西会变红。它与自主行同属「少给」一侧：
/// 判定者只产控制事件、**不得直接发言**（S07 §5）——没有这一行，`matrix_for` 会把主智能体
/// 整套能力（含 `reply.*`）派生给它，「判定者不得发言」在生产里被静默推翻，
/// 而文档与验收断言都还写着它成立。
pub(crate) const PRINCIPAL_REFLEX: &str = "session/reflex";

/// **对等主体**的身份（[roadmap/S08 §3](../../../docs/009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")）
/// ——表里那一行的名字与对等体写事件用的 `actor` 是同一个常量。
///
/// 它是本表里**第一个对等（图侧）主体**：与主从树的子智能体不同，对等体不依附某个会话，
/// 它的权威动作是**产生产物**（`produce.artifact`），协作经 `commitment.*` 事件而非
/// `chat.*`——故这一行**只有一项能力**，比主智能体窄得多。
///
/// **为什么必须登记**：未登记的 `session/<id>` 走 [`matrix_for`] 的派生分支，拿到主智能体
/// **整套**（含 `reply.*`）。S08 §4 的平凡值「身份分层 ≠ 权限分层」是给**子智能体**的
/// （它要能写自己的收束格，否则什么都写不了）；对等体拿整套则是**静默超权**——
/// 与 S12 为自主主体、S07 为判定者修掉的静默面同形。故 S08 §3.1 的纪律是：
/// **对等主体出现必须自带窄行同批落地，不许靠派生**（同 PL-009「闸不能晚于发起者」）。
pub(crate) const PRINCIPAL_PEER: &str = "session/peer-b";

/// **市场**的身份（[roadmap/S13 §3](../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")）——
/// 组织形态三角色的第一行，[T-073](../../../docs/009-task/073-多智能体社会数据行.md "T-073-多智能体社会数据行") 落地。
///
/// 只持 `assign.work`：市场的权威动作是**派活**（`agent_run` 的委托面），不是自己干活。
/// 可见域 `public`——市场公告（派给谁、什么活）是**公共域**的事实，T-133 §3 的
/// `vis_scope = public` 取值就钉在这一行上。
///
/// 与前三条窄行同形：主体出现必须自带窄行同批落地，未登记的 `session/market` 会走派生
/// 拿主智能体整套（含 `reply.*`）——「市场不得冒充主控发言」于是被静默推翻。
pub(crate) const PRINCIPAL_MARKET: &str = "session/market";

/// **仲裁者**的身份（[roadmap/S13 §3](../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")）——
/// 组织形态三角色的第二行，[T-073](../../../docs/009-task/073-多智能体社会数据行.md "T-073-多智能体社会数据行") 落地。
///
/// 只持 `assert.verification`：仲裁的权威动作是**对判定盖章**（`verdict.asserted`，
/// 淘汰回路的那一格）。C15「验证者 ≠ 产出者」是构造期约束——本行不持
/// `define.work` / `produce.artifact`，自成对安全；生产里唯一持验证能力的一行
/// （主智能体刻意不持，见本表 `session/main` 行注释）。
///
/// 可见域 `public`：仲裁裁决是**集体决策**的产物（S13 §1「集体决策 = 多个主体对同一
/// verdict 的竞争 + 仲裁者」），藏起来的裁决没有仲裁。
pub(crate) const PRINCIPAL_ARBITER: &str = "session/arbiter";

/// **守门者**的身份（[roadmap/S13 §3](../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")）——
/// 组织形态三角色的第三行，[T-073](../../../docs/009-task/073-多智能体社会数据行.md "T-073-多智能体社会数据行") 落地。
///
/// 只持 `judge.intent`：准入门槛的权威动作是**判来者意图**，与 [`PRINCIPAL_REFLEX`]
/// 同款独持先例（判定者不得直接发言）。可见域 `thread_private`：守门判定是判定者自己的
/// 工作记录（S13 §3「public 与 thread_private 并存」的另一半），不是公共域公告。
pub(crate) const PRINCIPAL_GATEKEEPER: &str = "session/gatekeeper";

/// **工人**的身份（[roadmap/S02 §3](../../../docs/009-task/122-工具调用与产物.md "T-122-工具调用与产物")）——
/// S02 worker 行，[T-107](../../../docs/009-task/107-角色主体执行路径批.md "T-107-角色主体执行路径批") 与执行路径同批落地。
///
/// 只持 `produce.artifact`：工人的权威动作是**产生产物**（`agent_run` 委派链上以产物
/// 收束的子主体，与对等行同形）。它**没有发言权**（不持 `reply.*`）——「以产物收束」
/// 的字面含义：这一轮的答话不入格，工具结果入格（收束闸的产物臂，[ADR-067](../../../docs/010-adr/067.md "ADR-067")）。
///
/// 与 [`PRINCIPAL_PEER`] 的两点不同，正是它必须**另立一行**的理由：对等体是**图侧**的
/// 协作方（`scope: root`，与主智能体平级），工人是**树侧**的委派落点（`agent_run` 派生的
/// 子会话，`scope: child`，受委派上限约束）；对等体经入站消息协作，工人经委派链驱动。
/// 窄的方向相同（只持产物能力）而角色不同——合并成一行会让「树侧委派」与「图侧协作」
/// 共享同一个主体名，而两者的驱动路径与上限判据都不同。
pub(crate) const PRINCIPAL_WORKER: &str = "session/worker";

/// **规划者**的身份（[roadmap/S03 §3](../../../docs/009-task/123-多步任务与返工.md "T-123-多步任务与返工")）——
/// S03 规划者行，[T-107](../../../docs/009-task/107-角色主体执行路径批.md "T-107-角色主体执行路径批") 落地（行本批落，驱动路径的裁决见该任务文档）。
///
/// 只持 `define.work`：规划的权威动作是**立任务**（任务图的开格与推进），与
/// [`PRINCIPAL_AUTONOMOUS`] 同款独持先例（自主发起者也只持它——「欲」升格为任务的那道
/// 闸后面写 `task.opened` 的就是这一项能力）。不持 `reply.*`：规划者不冒充用户对话。
///
/// 模式派生为 [`Pattern::Decider`](crate::symbio_core::Pattern)（无发言权而无产物能力
/// ⇒ 判据见 `Pattern::for_principal`）——与 S03 §3 的 ActorSpec 行 `pattern: decider`
/// 一致：规划步在该阶的规格里是**规则驱动**的（同 `AutonomousInitiator` 先例），
/// 不是又一个会说话的会话主体。
pub(crate) const PRINCIPAL_PLANNER: &str = "session/planner";

/// **验证者**的身份（[roadmap/S03 §3](../../../docs/009-task/123-多步任务与返工.md "T-123-多步任务与返工")）——
/// S03 验证者行，[T-107](../../../docs/009-task/107-角色主体执行路径批.md "T-107-角色主体执行路径批") 落地（行本批落，驱动路径的裁决见该任务文档）。
///
/// 只持 `assert.verification`：独立验证的权威动作是**对判定盖章**（S03 §2「独立验证者 =
/// 一行 ActorSpec，`capabilities: [AssertVerification]`」）。C15「验证者 ≠ 产出者」由
/// 构造期保证：本行不持 `define.work` / `produce.artifact`——**产出者不得自我认证**，
/// 与 [`PRINCIPAL_ARBITER`] 同一条纪律（那一行的裁决是声誉驱动的，这一行的判定是
/// 任务质量驱动的；两者同持验证能力而不同角色，故分行）。
///
/// 与仲裁行的可见域差异：裁决是**集体决策**的产物（public），验证判定是判定者自己的
/// 工作记录（thread_private）——与守门行同一条理由。
pub(crate) const PRINCIPAL_VERIFIER: &str = "session/verifier";

/// 本机会话**内容的属主**：`thread_private`（C10 缺省）下「谁能读到」等同于
/// 「是不是属主」，所以属主取值就是读侧闸的答案。
///
/// 今天它 = 本机用户（`[会话] ≈ [线程]`，S4 的 thread 实体落地后取线程属主）。
/// 属主是**部署事实**，与读方的 `principal` 一样不来自请求——请求方声明自己是谁
/// 已是上限，再声明「内容归谁」等于给自己授权。
pub(crate) const SESSION_OWNER: &str = PRINCIPAL_USER;

/// **外部执行**能力名（`ProduceArtifact` 的书写形态）——熔断闸门的授权判据
/// （[roadmap/S09 §6](../../../docs/009-task/129-外部执行与熔断.md "T-129-外部执行与熔断") 验收 1）。
///
/// 提成常量而不是在闸门那里再写一遍字面量：能力名只有一个拼法，写两处迟早漂移成
/// 「表授予 A、闸门判 B」，而这种错位**没有任何东西会变红**（两边都是合法能力名）。
pub(crate) const CAP_EXTERNAL_EXECUTION: &str = "produce.artifact";

/// **指派工作**能力名（`AssignWork` 的书写形态）——淘汰回路收回的那一项
/// （[roadmap/S13 §2](../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")
/// 「淘汰 = 声誉投影低于阈值 → 不再被授予 `assign.work`」）。
///
/// 与 [`CAP_EXTERNAL_EXECUTION`] 同一条理由提成常量：这个名字出现在三个地方——
/// 授权表行（`session/market` / `session/main`）、淘汰裁决的载荷（`verdict.asserted`
/// 的 `capability` 字段）与闸门的撤销判据（`agent_run` 前问覆盖层）——三处各写
/// 一遍字面量，迟早漂移成「裁决收回 A、闸门判 B」。
pub(crate) const CAP_ASSIGN_WORK: &str = "assign.work";

/// **立任务**能力名（`DefineWork` 的书写形态）——任务表写侧闸的授权判据
/// （[roadmap/S03 §3](../../../docs/009-task/123-多步任务与返工.md "T-123-多步任务与返工")
/// 「规划者 ▸ `define.work`」，[T-107](../../../docs/009-task/107-角色主体执行路径批.md "T-107-角色主体执行路径批") 落地其读方）。
///
/// 与 [`CAP_ASSIGN_WORK`] 同一条理由提成常量：这个名字今天出现在四个地方——授权表行
/// （`session/main` / `session/autonomous` / `session/planner`）、`AutonomousInitiator` 写
/// `task.opened` 的那项能力、任务表写侧闸的放行判据、与 `assert.verification` 配对时
/// C15 的构造期检查。四处各写一遍字面量，迟早漂移成「表授予 A、闸门判 B」。
pub(crate) const CAP_DEFINE_WORK: &str = "define.work";

/// **独立验证**能力名（`AssertVerification` 的书写形态）——任务表写侧闸的另一半放行
/// 判据（[T-107](../../../docs/009-task/107-角色主体执行路径批.md "T-107-角色主体执行路径批")）。
///
/// 提成常量的理由同 [`CAP_DEFINE_WORK`]：它出现在授权表行（`session/arbiter` /
/// `session/verifier`）、淘汰裁决的写侧闸（`EliminationDecider`，T-073）、任务表写侧闸
/// 的放行判据与 C15 构造期检查共四处。
pub(crate) const CAP_ASSERT_VERIFICATION: &str = "assert.verification";

/// 本机主体清单：`(主体, 能力名, 可见域)`。能力名的闭集与拼写由
/// [`PermissionMatrix::from_names`] 校验（认不出即拒绝构造）。
///
/// 十三行，各自存在的理由不同：
///
/// - `session/main`（主智能体）持 6 项，**刻意不持 `assert.verification`**：C15 不自验
///   ——验证者 ≠ 产出者是构造期约束（`PermissionMatrix::new` 会拒绝双持），本机目前
///   没有独立验证者行，验证能力因此无人持有；
/// - [`PRINCIPAL_AUTONOMOUS`]（自主发起者）**只持 `define.work`**——自主行为不得冒充
///   用户对话（S12 §3 grants 行 / §5）；
/// - [`PRINCIPAL_REFLEX`]（抢占判定者）**只持 `judge.intent`**——判定者不得直接发言
///   （S07 §5）：它写的是控制事件（`task.held` / `task.progressed` / `control.opened`），
///   不含任何 `reply.*`；
/// - [`PRINCIPAL_PEER`]（对等主体）**只持 `produce.artifact`**——对等体是图侧的协作方，
///   权威动作是产生产物（S08 §3 的 ActorSpec 行 `capabilities: [ProduceArtifact]`），
///   协作经 `commitment.*` 而非 `chat.*`；
/// - [`PRINCIPAL_MARKET`]（市场）**只持 `assign.work`**，可见域 `public`——派活是市场的
///   权威动作，公告是公共域事实（S13 §3，T-073）；
/// - [`PRINCIPAL_ARBITER`]（仲裁者）**只持 `assert.verification`**，可见域 `public`——
///   验证能力在本表的持有者之一；裁决是集体决策的产物（S13 §1/§3，T-073）；
/// - [`PRINCIPAL_GATEKEEPER`]（守门者）**只持 `judge.intent`**，可见域 `thread_private`——
///   准入门槛判来者意图，与判定行同款独持（S13 §3，T-073）；
/// - [`PRINCIPAL_WORKER`]（工人）**只持 `produce.artifact`**——树侧委派链上以产物收束的
///   子主体（S02 worker 行，T-107）：与对等行窄的方向相同而角色不同（树侧委派 vs 图侧协作），
///   故另立一行；它的驱动路径 = `agent_run` 委派（子会话绑 `worker` ⇒ 主体 `session/worker`
///   ⇒ 工具结果落 `artifact.added`，作者是它自己）；
/// - [`PRINCIPAL_PLANNER`]（规划者）**只持 `define.work`**——立任务是规划者的权威动作
///   （S03 §3，T-107）：与自主行同款独持（`AutonomousInitiator` 的先例），模式派生为
///   `Decider`（规则驱动的规划步，不是会说话的会话主体）；
/// - [`PRINCIPAL_VERIFIER`]（验证者）**只持 `assert.verification`**——独立验证的权威动作
///   是对判定盖章（S03 §3，T-107）：与仲裁行同持验证能力而不同角色（任务质量判定 vs
///   声誉裁决），C15 由构造期保证（本行不持 `define.work` / `produce.artifact`）。
///
/// 后九行都是本表里**比主智能体窄**的记录。窄的方向相同而理由各异：子智能体怕的是
/// **写不了**（要能写自己的收束格），自主行怕冒充用户对话，判定行怕抢在编排层之前发言，
/// 对等行怕的是**以本机主控的身份行事**（图侧协作 ≠ 本机主控），市场行怕的是**自己下场干活**
/// （派活 ≠ 干活），仲裁行怕的是**自证**（C15），守门行怕的是**先发声后判**，工人行怕的是
/// **以主控身份发言**（产物收束 ≠ 发言），规划行怕的是**自验**（C15：不持验证能力），
/// 验证行怕的是**既产出又认证**（C15：不持产出能力）。
///
/// 少了这些行会怎样（几批修掉的正是这个）：未登记的 `session/<id>` 走 [`matrix_for`]
/// 的派生分支，拿到主智能体的**整套**能力集（含 `reply.*`）——于是「自主行为不得冒充
/// 用户对话」「判定者不得发言」「对等体只产生产物」「市场只派活」在生产里被静默推翻，
/// 而文档与验收断言都还写着它们成立。
const ROWS: &[(&str, &[&str], VisScope)] = &[
    (
        PRINCIPAL_MAIN,
        &[
            "judge.intent",
            "reply.first",
            "reply.append",
            "define.work",
            CAP_EXTERNAL_EXECUTION,
            "assign.work",
        ],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_AUTONOMOUS,
        &["define.work"],
        VisScope::ThreadPrivate,
    ),
    (PRINCIPAL_REFLEX, &["judge.intent"], VisScope::ThreadPrivate),
    (
        PRINCIPAL_PEER,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
    // 组织形态三角色（S13 §3，T-073）：市场派活（公共域）/ 仲裁盖章（公共域）/ 守门判意图（私有）。
    // 三行都是**单能力独持**——与自主 / 判定 / 对等三行同一条纪律：角色出现必须自带窄行，
    // 不许靠派生拿主智能体整套。arbiter 持 `assert.verification` 不碰 C15（不持
    // `define.work` / `produce.artifact`，构造期双持检查见 `governance`）。
    (PRINCIPAL_MARKET, &["assign.work"], VisScope::Public),
    (
        PRINCIPAL_ARBITER,
        &["assert.verification"],
        VisScope::Public,
    ),
    (
        PRINCIPAL_GATEKEEPER,
        &["judge.intent"],
        VisScope::ThreadPrivate,
    ),
    // S02/S03 三角色行（T-107，行与执行路径同批）：工人产生产物（树侧委派收束）、
    // 规划者立任务、验证者独立判定。三行都是**单能力独持**——与自主 / 判定 / 对等 /
    // 市场 / 仲裁 / 守门六行同一条纪律：角色出现必须自带窄行，不许靠派生拿主智能体
    // 整套。planner 持 `define.work` 不碰 C15（不持 `assert.verification`）；verifier
    // 持 `assert.verification` 亦不碰 C15（不持 `define.work` / `produce.artifact`，
    // 构造期双持检查见 `governance`）——「产出者不得自我认证」两边都成立。
    (
        PRINCIPAL_WORKER,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_PLANNER,
        &[CAP_DEFINE_WORK],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_VERIFIER,
        &[CAP_ASSERT_VERIFICATION],
        VisScope::ThreadPrivate,
    ),
    // 三族工具主体行（ADR-069 §决策 2 工具响应主体，T-146 落地）：工具的权威动作是
    // **产生产物**（`produce.artifact`）——工具响应的正文就是一条 artifact。与
    // `session/worker` / `session/peer-b` 同款独持（窄的方向相同而角色不同：worker 是
    // 树侧委派落点、peer 是图侧协作方、tool 是**能力**发言方）。今天写侧闸按会话
    // 主体判，这三行是**声明性登记**，为 T-147 之后 `plugin::attributed` 按 kind=Tool
    // 补 principal 铺好底。缺这一行不会红——表里没它 ⇒ `matrix_for` 走「非 agent 命名
    // 空间 ⇒ 直接用生产表」的分支 ⇒ `can_produce_artifact(vdfs/write)` fail-closed；
    // 今天的写入点都按 session principal 判，不撞这条。
    (
        PRINCIPAL_TOOL_VDFS_WRITE,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_TOOL_WEB_FETCH,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_TOOL_CONTENT_SEARCH,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
];

static MATRIX: OnceLock<PermissionMatrix> = OnceLock::new();

/// 本机会话的**主体派生**（部署事实 → [`PRINCIPAL_MAIN`] 同族的取值点）：
/// 会话选定了哪个 agent，这轮所有断言就是哪个主体；未选 ⇒ 主智能体。
///
/// 判据取自 core 的可见域约定 [`is_agent_id`](crate::symbio_core::view::is_agent_id)
/// 与 [`principal_namespace`]（[ADR-069 §决策 3](../../../docs/010-adr/069.md "ADR-069")）——
/// 「什么地址算智能体一族」只定义一次，可见域判据与身份派生共用登记表，
/// 两处各写一个字面量迟早漂移成「判的是 A、写的是 B」。
///
/// 平凡值（S08 §4）：今天所有会话未选智能体 ⇒ 恒为 `session/main`，与
/// [`PRINCIPAL_MAIN`] **逐字一致**（收束事件的 `actor` 取的正是它）。
pub(crate) fn principal_of(agent_id: Option<&str>) -> String {
    match agent_id.map(str::trim).filter(|s| !s.is_empty()) {
        None => PRINCIPAL_MAIN.to_string(),
        // 已是可路由地址 `<ns>/<name>` 的主体名就原样认下——派生点不二次加 namespace。
        // 未带 namespace 的裸名走 session 归属（本会话主体一族）；person/tool/system
        // 三类由调用方给出完整地址（`person/local` / `vdfs/write` / `system/compactor`），
        // 不会走到这里。
        Some(id) if principal_namespace(id).is_some() => id.to_string(),
        Some(id) => format!("session/{id}"),
    }
}

/// 一次**请求上下文**的主体身份——全 crate 唯一读 `ctx[AGENT_ID]` 并派生主体名的取值点。
///
/// 派生本身是 [`principal_of`]（部署事实的唯一 owner），这里只多一层「从请求上下文取
/// 那个 id」。写事件的 `actor`、写消息的 `principal`、写侧闸判的对象三者都走它——
/// 三处各推一次迟早漂移成「判的是 A、写的是 B」。
///
/// 为什么住 core 而不留会话插件（[ADR-082](../../../docs/010-adr/082.md "ADR-082") 裁决三 1 组）：
/// 它是对 core 既有键（`AGENT_ID`）的**纯派生函数**，不属于任何一个切片；会话驱动器
/// 切成独立插件后（[plan/14 步 6](../../../docs/004-plan/014-单环一表收敛.md "PL-014")），
/// 读侧（`session/stats`）与写侧隔着 E-009 的插件墙两头都要用，留在任一侧都会变成
/// 另一条反向引用。
pub(crate) fn request_principal(ctx: &dyn crate::symbio_core::PluginInvokeRequest) -> String {
    use crate::symbio_core::PluginInvokeRequestExt;
    principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref())
}

/// **该主体当次判定**用的矩阵（行的形状不变：`(主体, 能力名, 可见域)`，
/// 见本模块文档「本表会怎么长」）。
///
/// 两条分支，先表后派生：
///
/// 1. **表里登记过的主体**（主智能体、自主发起者、抢占判定者、对等主体、市场 / 仲裁 / 守门、工人 / 规划者 / 验证者、以及工具与系统三行）与
///    **非 agent 命名空间**的主体（`person/*` / `vdfs/*` / `web/*` / `system/*` 及一切
///    未登记名）都直接用[生产表](Self::production_matrix)：登记过的拿自己那一行
///    ——**表是权威，派生不得覆盖它**（覆盖就成「表授予 A、闸门判 B」）；未登记的
///    `person/local` 在表里没有行 ⇒ `can_reply` 为假 ⇒ fail-closed。少一行是一条**拒绝**，
///    不是一条漏判。
/// 2. **未登记的 `session/<id>`**（子智能体 / 对等体）派生：要能写自己的收束格，就必须
///    有自己那一行。行的**内容** = **主智能体**那一行的能力集——**身份分层 ≠ 权限分层**，
///    本机所有智能体同角色（S08 §4 平凡值）；按主体展开 grants 做能力分级是 S13 的工序。
///
/// 取的是主智能体那一行、**不是「第一行」**：表加了一行就把派生口径换掉，等于给行序
/// 赋予了语义，而没有任何东西会为此变红。
pub(crate) fn matrix_for(principal: &str) -> Cow<'static, PermissionMatrix> {
    let live = production_matrix();
    if live.sees_of(principal).is_some() || !is_agent_id(principal) {
        return Cow::Borrowed(live);
    }
    let (main_caps, main_scope) = ROWS
        .iter()
        .find(|(p, _, _)| *p == PRINCIPAL_MAIN)
        .map(|(_, caps, scope)| (*caps, *scope))
        .unwrap_or((&[], VisScope::ThreadPrivate));
    Cow::Owned(
        PermissionMatrix::from_names(&[(principal, main_caps, main_scope)]).unwrap_or_else(|why| {
            crate::plugin_warn!(
                "authz",
                "[governance] 主体 `{principal}` 的行构造失败，按 fail-closed 降级为空矩阵：{why}"
            );
            PermissionMatrix::from_names(&[]).expect("空授权表无主体可校验，不可能违反成对性")
        }),
    )
}

/// 本机授权矩阵（懒构造一次；构造失败 ⇒ 空矩阵 = 谁都不许，并告警）。
///
/// 写侧收束、读侧可见域与外部执行闸门的授权判据都取这一份，保证「表只有一张」——
/// 各判各的就会漂移成「闸放行、表说没有」。
pub(crate) fn production_matrix() -> &'static PermissionMatrix {
    MATRIX.get_or_init(|| {
        PermissionMatrix::from_names(ROWS).unwrap_or_else(|why| {
            crate::plugin_warn!(
                "authz",
                "[governance] 授权表构造失败，按 fail-closed 降级为空矩阵（{}）：{}",
                ROWS.len(),
                why
            );
            PermissionMatrix::from_names(&[]).expect("空授权表无主体可校验，不可能违反成对性")
        })
    })
}

#[cfg(test)]
#[path = "authz.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/authz.rs::matrix_for: 穷尽性不变量：上游已穷举 / 已校验，走到 else 说明本文件的判据漏了一个分支，属代码缺陷。
// panic-allow symbio/src/symbio_core/authz.rs::production_matrix: 穷尽性不变量：上游已穷举 / 已校验，走到 else 说明本文件的判据漏了一个分支，属代码缺陷。
