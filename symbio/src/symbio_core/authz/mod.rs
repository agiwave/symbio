//! 本机部署的**授权策略表**（写侧 grants + 读侧可见域，[plan/01 §7](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 为什么这张表住 `symbio_core`
//!
//! 表里写的是**部署事实**——本机有哪几个主体、各持有什么能力。三方的边界是：
//!
//! | 谁 | 拥有 | 不拥有 |
//! |---|---|---|
//! | 同处 core 的 `governance` | 能力闭集（成员数由 DE-001 §7 封顶）、成对性与两条判定的**机制** | 「谁」是部署事实 |
//! | 本模块 | 主体清单与各自的 grants / 可见域 | 判定语义（复用 `governance` 的函数） |
//! | 各插件 | 自己那道闸上「现在该判谁」 | 表本身——插件互不可见（E-009），任一插件持有它都会逼别的插件跨插件引用 |
//!
//! 表是**架构元素**：「谁持有什么」与「能不能」是同一张表的两半，分家的代价是
//! 「表授予 A、闸门判 B」这种错位**没有任何东西会变红**——两边都是合法能力名，
//! 事后没有任何东西能指出是哪半边写错了。故它与机制同住一棵模块树，
//! 整块经 `pub(crate)` 开给全 crate（见 `symbio_core/mod.rs`）：
//! 跨插件可见，却不进对外公开面——表里写的是部署事实，不是契约。
//!
//! ## fail-closed 三层
//!
//! 1. **构造期**：认不出的能力名 ⇒ 拒绝整体构造（`PolicyError`，core 定义），
//!    降级为**空矩阵**并告警——不是「跳过那一行」，笔误不能变成一条悄悄生效的拒绝；
//! 2. **写侧**：运行器落格前判（`turn_runner::closure_gate`，按**种类**分两臂）：
//!    发言收束（收束格）判 `can_reply`，产物收束
//!    （`artifact.added`）判 `can_produce_artifact`
//!    （[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")）。未持有 ⇒ **不落那一格，
//!    但落一条否定事实** `authz.denied{key, kind, why}`：
//!    发言臂下该轮留在未收束态——除非它落过产物格（C4 与产物闸同批认可「产物即收束」，
//!    见 `invariants::unresolved_turns`）。
//!
//!    **「拒绝 = 不落格」是旧口径，已订正为「拒绝 = 落 `denied` 且不落格」。**
//!    差别不是记账习惯：不落 `denied` 时，「被拒」与「根本没跑完」在日志上同形，
//!    事后答不出「谁在什么时候因为什么被拒了」，回放也重现不出这次拒绝。
//!    这违反的是 I1（单通道）——「被拒绝」是一个状态变更，它也必须是一条事实。
//! 3. **读侧**：`stats` 按读方声明判 `can_see`——属主
//!    是**内容的属主**（`SESSION_OWNER`，部署事实）而非事件作者，于是今天的判读
//!    就是「属主放行、其余整体拒绝」；`thread_private` 缺省（C10）。
//!
//! ## 两张表里的「人」
//!
//! `user.message` **不入能力表**：能力按「智能体的权威动作种类」划分，人的发言
//! 不是动作种类（落不进去就走 [03 §3](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的 ADR）。
//! 读侧因此也不靠矩阵成员身份，而靠 **owner 相等**：`thread_private` 下属主是
//! 本机会话的属主，语义见 `governance::can_see`。
//!
//! ## 本表会怎么长
//!
//! 多主体加固（plan/11 批 2 /
//! 04 §3.1 批⑧）落地时，子智能体各成一行——行的形状不变，仍是
//! `(主体, 能力名, 可见域)`。
//!
//! 自主发起者那一行已按 [S12 §3](../../../../docs/001-goal/132-自主层与长期目标.md "GO-132-自主层与长期目标")
//! 先落地：它是本表里**第一行「比主智能体窄」**的记录（只持 `define.work`）。这行同时
//! 说明一件事——**窄行是唯一入口**（[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")）：
//! 未登记的主体在表里没有行 ⇒ 每个谓词为假，登记过的一行就是它的权威能力集，
//! 谁都不派生谁。
//!
//! 抢占判定者那一行（`session/reflex`，只持 `judge.intent`）按 [S07 §5](../../../../docs/001-goal/127-插话与实时打断.md "GO-127-插话与实时打断")
//! 随后落地，与自主行同形：**正文写着「不得直接发言」而表里没有行**时，判据是
//! 「没有行 = 拒」——断言与表同一方向。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::symbio_core::governance::{Capability, PermissionMatrix};
use crate::symbio_core::{principal_namespace, VisScope};

/// 本机主智能体的身份——**表里那一行的名字**与收束事件的 `actor`
/// 是同一个常量：授权判定的对象若与实际写入的主体各写一份字符串，两边迟早漂移
/// 到「判的是 A、写的是 B」而没有任何东西会变红。
///
/// id 是**可路由地址**（[D-069 §决策 2](../../../../docs/003-design/005-协议规范.md#d-069-principal-三字段kind-与-id-分家id-是可路由地址 "DE-005-协议规范")）：
/// `<namespace>/<name>`——`session` 是本机智能体的归属 namespace。
pub(crate) const PRINCIPAL_MAIN: &str = "session/main";

/// 本机用户的身份——用户发言事件的 `actor`。
///
/// [D-069] 把它从裸字面量 `user` 归一为可路由地址 `person/local`——
/// `person` 是概念 namespace（无插件归属时用概念名）。语义不变：`user.message`
/// **不入能力表**（能力按「智能体的权威动作种类」划分，人的发言不是动作种类），
/// 本表**不为它立行**——表里没有它的行 ⇒ `can_reply` fail-closed（[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")）。
pub(crate) const PRINCIPAL_USER: &str = "person/local";

/// **vdfs 写工具**的身份（[D-069 §决策 2]）——工具响应叶子的 `principal`；
/// 与调用它的 `session/<caller>` 平级（工具请求节点仍归调用者）。id 的 namespace 是插件名。
///
/// 本表给它一行：`produce.artifact`——工具的权威动作是**产生产物**
/// （`artifact.added` 的响应正文），与 `session/worker` / `session/peer-b` 同款独持。
/// 今天写侧闸按会话主体判，不按工具主体判，故这一行是**声明性**的——它把「工具
/// 是主体」写进表；[] 之后 `plugin::attributed` 会按 kind=Tool 补上这个
/// principal 值，判据不变。
pub(crate) const PRINCIPAL_TOOL_VDFS_WRITE: &str = "vdfs/write";

/// **网页取回工具**的身份。
pub(crate) const PRINCIPAL_TOOL_WEB_FETCH: &str = "web/fetch";

/// **本地检索工具**的身份。
pub(crate) const PRINCIPAL_TOOL_CONTENT_SEARCH: &str = "local/content_search";

/// **压缩器**系统主体（[D-069 §决策 2]）——compaction 事件的将来发言方。
///
/// 这里**不立 Rust 常量**：今天压缩节点由会话主体落、写侧没有消费方，`dead_code`
/// 会逐条点名，而 R-002 的棘轮不许为「将来会用」加承认标记——常量的唯一 owner 是
/// [D-069 §决策 2] 那张表，写侧接线时（[] 把「压缩由谁写」的判据换掉）随接线
/// 一批立常量、随布线摘标记，两步同批发生。
///
/// 自主发起者的身份（[roadmap/S12 §3](../../../../docs/001-goal/132-自主层与长期目标.md "GO-132-自主层与长期目标")）
/// ——表里那一行的名字与 `AutonomousInitiator` 四个事件的 `actor` 是同一个常量。
///
/// 与 [`PRINCIPAL_MAIN`] 同理：判定用的主体名与实际写入的主体名各写一份字符串，两边
/// 迟早漂移到「判的是 A、写的是 B」而没有任何东西会变红——**这里尤其致命**，因为这一行
/// 存在的全部理由就是**少给**（它不持 `reply.*`）；名字对不上，那行就成了一条谁也管不到的
/// 死记录，而自主行为照旧能冒充用户对话。
pub(crate) const PRINCIPAL_AUTONOMOUS: &str = "session/autonomous";

/// 抢占判定者的身份（[roadmap/S07 §3](../../../../docs/001-goal/127-插话与实时打断.md "GO-127-插话与实时打断")）
/// ——表里那一行的名字与 `PreemptionDecider` 三个事件（`task.held` /
/// `task.progressed` / `control.opened`）的 `actor` 是同一个常量。
///
/// 与 [`PRINCIPAL_AUTONOMOUS`] 同理：判定用的主体名与实际写入的主体名各写一份字符串，
/// 两边迟早漂移到「判的是 A、写的是 B」而没有任何东西会变红。它与自主行同属「少给」一侧：
/// 判定者只产控制事件、**不得直接发言**（S07 §5）——这一行就是它的全部能力来源，
/// 「不得发言」与断言同方向（D-099：表是权威，未登记 = 拒）。
pub(crate) const PRINCIPAL_REFLEX: &str = "session/reflex";

/// **对等主体**的身份（[roadmap/S08 §3](../../../../docs/001-goal/128-多主体与对等承诺.md "GO-128-多主体与对等承诺")）
/// ——表里那一行的名字与对等体写事件用的 `actor` 是同一个常量。
///
/// 它是本表里**第一个对等（图侧）主体**：与主从树的子智能体不同，对等体不依附某个会话，
/// 它的权威动作是**产生产物**（`produce.artifact`），协作经 `commitment.*` 事件而非
/// `chat.*`——故这一行**只有一项能力**，比主智能体窄得多。
///
/// **为什么必须登记**：未登记的 `session/<id>` 在表里没有行 ⇒ 每个谓词为假
/// （[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")）；登记过的行就是它的权威能力集。
/// S08 §3.1 的纪律是：**对等主体出现必须自带窄行同批落地**（同 设计文档「闸不能晚于
/// 发起者」）——窄行不是补丁，是它唯一的入口。
pub(crate) const PRINCIPAL_PEER: &str = "session/peer-b";

/// **市场**的身份（[roadmap/S13 §3](../../../../docs/001-goal/133-多智能体社会.md "GO-133-多智能体社会")）——
/// 组织形态三角色的第一行， 落地。
///
/// 只持 `assign.work`：市场的权威动作是**派活**（`agent_run` 的委托面），不是自己干活。
/// 可见域 `public`——市场公告（派给谁、什么活）是**公共域**的事实， §3 的
/// `vis_scope = public` 取值就钉在这一行上。
///
/// 与前三条窄行同形：主体出现必须自带窄行同批落地，未登记的 `session/market` 会走派生
/// 拿主智能体整套（含 `reply.*`）——「市场不得冒充主控发言」于是被静默推翻。
pub(crate) const PRINCIPAL_MARKET: &str = "session/market";

/// **仲裁者**的身份（[roadmap/S13 §3](../../../../docs/001-goal/133-多智能体社会.md "GO-133-多智能体社会")）——
/// 组织形态三角色的第二行， 落地。
///
/// 只持 `assert.verification`：仲裁的权威动作是**对判定盖章**（`verdict.asserted`，
/// 淘汰回路的那一格）。C15「验证者 ≠ 产出者」是构造期约束——本行不持
/// `define.work` / `produce.artifact`，自成对安全；生产里唯一持验证能力的一行
/// （主智能体刻意不持，见本表 `session/main` 行注释）。
///
/// 可见域 `public`：仲裁裁决是**集体决策**的产物（S13 §1「集体决策 = 多个主体对同一
/// verdict 的竞争 + 仲裁者」），藏起来的裁决没有仲裁。
pub(crate) const PRINCIPAL_ARBITER: &str = "session/arbiter";

/// **守门者**的身份（[roadmap/S13 §3](../../../../docs/001-goal/133-多智能体社会.md "GO-133-多智能体社会")）——
/// 组织形态三角色的第三行， 落地。
///
/// 只持 `judge.intent`：准入门槛的权威动作是**判来者意图**，与 [`PRINCIPAL_REFLEX`]
/// 同款独持先例（判定者不得直接发言）。可见域 `thread_private`：守门判定是判定者自己的
/// 工作记录（S13 §3「public 与 thread_private 并存」的另一半），不是公共域公告。
pub(crate) const PRINCIPAL_GATEKEEPER: &str = "session/gatekeeper";

/// **工人**的身份（[roadmap/S02 §3](../../../../docs/001-goal/122-工具调用与产物.md "GO-122-工具调用与产物")）——
/// S02 worker 行， 与执行路径同批落地。
///
/// 只持 `produce.artifact`：工人的权威动作是**产生产物**（`agent_run` 委派链上以产物
/// 收束的子主体，与对等行同形）。它**没有发言权**（不持 `reply.*`）——「以产物收束」
/// 的字面含义：这一轮的答话不入格，工具结果入格（收束闸的产物臂，[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")）。
///
/// 与 [`PRINCIPAL_PEER`] 的两点不同，正是它必须**另立一行**的理由：对等体是**图侧**的
/// 协作方（`scope: root`，与主智能体平级），工人是**树侧**的委派落点（`agent_run` 派生的
/// 子会话，`scope: child`，受委派上限约束）；对等体经入站消息协作，工人经委派链驱动。
/// 窄的方向相同（只持产物能力）而角色不同——合并成一行会让「树侧委派」与「图侧协作」
/// 共享同一个主体名，而两者的驱动路径与上限判据都不同。
pub(crate) const PRINCIPAL_WORKER: &str = "session/worker";

/// **规划者**的身份（[roadmap/S03 §3](../../../../docs/001-goal/123-多步任务与返工.md "GO-123-多步任务与返工")）——
/// S03 规划者行， 落地（行本批落，驱动路径的裁决见该任务文档）。
///
/// 只持 `define.work`：规划的权威动作是**立任务**（任务图的开格与推进），与
/// [`PRINCIPAL_AUTONOMOUS`] 同款独持先例（自主发起者也只持它——「欲」升格为任务的那道
/// 闸后面写 `task.opened` 的就是这一项能力）。不持 `reply.*`：规划者不冒充用户对话。
///
/// 模式派生为 `Pattern::Decider`（无发言权而无产物能力
/// ⇒ 判据见 `Pattern::for_principal`）——与 S03 §3 的 ActorSpec 行 `pattern: decider`
/// 一致：规划步在该阶的规格里是**规则驱动**的（同 `AutonomousInitiator` 先例），
/// 不是又一个会说话的会话主体。
pub(crate) const PRINCIPAL_PLANNER: &str = "session/planner";

/// **验证者**的身份（[roadmap/S03 §3](../../../../docs/001-goal/123-多步任务与返工.md "GO-123-多步任务与返工")）——
/// S03 验证者行， 落地（行本批落，驱动路径的裁决见该任务文档）。
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

/// **读侧闸那一条判据的唯一写法**：`viewer` 能不能看 `scope` 这一档内容。
///
/// 两支同属一次判定（不是两个判定点）：
/// - **属主恒放行**（owner 相等，C10 的读侧口径）——`user` 在矩阵里没有行
///   （能力表按「智能体的权威动作种类」划分），`shared` 的「矩阵内主体可见」若不叠
///   这一条，属主本人会被**自己共享的内容**挡在门外：那是把「本人写、智能体读」
///   读成「本人也不能读」；
/// - 其余走 [`PermissionMatrix::can_see`]（矩阵内放行、矩阵外必拒，
///   fail-closed——「没说允许」永远不等于允许）。
///
/// ## 为什么它必须是唯一的
///
/// 这条判据今天有**两个消费方**：`session/stats` 的整列闸（一份事实源一个答案）与
/// 召回的逐条预过滤（`plugins/loop/memory_events.rs`，跨会话切片逐条判）。两边各自的
/// 那一层**作用域**差别是有意保留的（整列闸要表达「未声明读方 = 全量」，召回要表达
/// 「别处的 `thread_private` 连属主也不进召回」），但**放宽的那两支必须同一条**——
/// 各写一遍就会漂成「stats 给看、召回不给看」这类没有理由的半开半掩（②、 ⑧）。
pub(crate) fn content_visible(scope: VisScope, viewer: &str) -> bool {
    viewer == SESSION_OWNER || production_matrix().can_see(viewer, SESSION_OWNER, scope)
}

/// **外部执行**能力名（`ProduceArtifact` 的书写形态）——熔断闸门的授权判据
/// （[roadmap/S09 §6](../../../../docs/001-goal/129-外部执行与熔断.md "GO-129-外部执行与熔断") 验收 1）。
///
/// 提成常量而不是在闸门那里再写一遍字面量：能力名只有一个拼法，写两处迟早漂移成
/// 「表授予 A、闸门判 B」，而这种错位**没有任何东西会变红**（两边都是合法能力名）。
/// 拼法本身住在 `governance` 的声明表里（[`Capability::name`]）——本常量不重复它，
/// 只给那道闸一个**语义名**（⑥）。
pub(crate) const CAP_EXTERNAL_EXECUTION: &str = Capability::ProduceArtifact.name();

/// **指派工作**能力名（`AssignWork` 的书写形态）——淘汰回路收回的那一项
/// （[roadmap/S13 §2](../../../../docs/001-goal/133-多智能体社会.md "GO-133-多智能体社会")
/// 「淘汰 = 声誉投影低于阈值 → 不再被授予 `assign.work`」）。
///
/// 与 [`CAP_EXTERNAL_EXECUTION`] 同一条理由提成常量：这个名字出现在三个地方——
/// 授权表行（`session/market` / `session/main`）、淘汰裁决的载荷（`verdict.asserted`
/// 的 `capability` 字段）与闸门的撤销判据（`agent_run` 前问覆盖层）——三处各写
/// 一遍字面量，迟早漂移成「裁决收回 A、闸门判 B」。
pub(crate) const CAP_ASSIGN_WORK: &str = Capability::AssignWork.name();

/// **立任务**能力名（`DefineWork` 的书写形态）——任务表写侧闸的授权判据
/// （[roadmap/S03 §3](../../../../docs/001-goal/123-多步任务与返工.md "GO-123-多步任务与返工")
/// 「规划者 ▸ `define.work`」， 落地其读方）。
///
/// 与 [`CAP_ASSIGN_WORK`] 同一条理由提成常量：这个名字今天出现在四个地方——授权表行
/// （`session/main` / `session/autonomous` / `session/planner`）、`AutonomousInitiator` 写
/// `task.opened` 的那项能力、任务表写侧闸的放行判据、与 `assert.verification` 配对时
/// C15 的构造期检查。四处各写一遍字面量，迟早漂移成「表授予 A、闸门判 B」。
pub(crate) const CAP_DEFINE_WORK: &str = Capability::DefineWork.name();

/// **独立验证**能力名（`AssertVerification` 的书写形态）——任务表写侧闸的另一半放行
/// 判据。
///
/// 提成常量的理由同 [`CAP_DEFINE_WORK`]：它出现在授权表行（`session/arbiter` /
/// `session/verifier`）、淘汰裁决的写侧闸（`EliminationDecider`，）、任务表写侧闸
/// 的放行判据与 C15 构造期检查共四处。
pub(crate) const CAP_ASSERT_VERIFICATION: &str = Capability::AssertVerification.name();

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
///   权威动作，公告是公共域事实（S13 §3，）；
/// - [`PRINCIPAL_ARBITER`]（仲裁者）**只持 `assert.verification`**，可见域 `public`——
///   验证能力在本表的持有者之一；裁决是集体决策的产物（S13 §1/§3，）；
/// - [`PRINCIPAL_GATEKEEPER`]（守门者）**只持 `judge.intent`**，可见域 `thread_private`——
///   准入门槛判来者意图，与判定行同款独持（S13 §3，）；
/// - [`PRINCIPAL_WORKER`]（工人）**只持 `produce.artifact`**——树侧委派链上以产物收束的
///   子主体（S02 worker 行，）：与对等行窄的方向相同而角色不同（树侧委派 vs 图侧协作），
///   故另立一行；它的驱动路径 = `agent_run` 委派（子会话绑 `worker` ⇒ 主体 `session/worker`
///   ⇒ 工具结果落 `artifact.added`，作者是它自己）；
/// - [`PRINCIPAL_PLANNER`]（规划者）**只持 `define.work`**——立任务是规划者的权威动作
///   （S03 §3，）：与自主行同款独持（`AutonomousInitiator` 的先例），模式派生为
///   `Decider`（规则驱动的规划步，不是会说话的会话主体）；
/// - [`PRINCIPAL_VERIFIER`]（验证者）**只持 `assert.verification`**——独立验证的权威动作
///   是对判定盖章（S03 §3，）：与仲裁行同持验证能力而不同角色（任务质量判定 vs
///   声誉裁决），C15 由构造期保证（本行不持 `define.work` / `produce.artifact`）。
///
/// 后九行都是本表里**比主智能体窄**的记录。窄的方向相同而理由各异：子智能体怕的是
/// **写不了**（要能写自己的收束格），自主行怕冒充用户对话，判定行怕抢在编排层之前发言，
/// 对等行怕的是**以本机主控的身份行事**（图侧协作 ≠ 本机主控），市场行怕的是**自己下场干活**
/// （派活 ≠ 干活），仲裁行怕的是**自证**（C15），守门行怕的是**先发声后判**，工人行怕的是
/// **以主控身份发言**（产物收束 ≠ 发言），规划行怕的是**自验**（C15：不持验证能力），
/// 验证行怕的是**既产出又认证**（C15：不持产出能力）。
///
/// 少了这些行会怎样（[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范") 起）：**该主体什么
/// 都做不了**——未登记主体在表里没有行，每个谓词为假（fail-closed）。「忘配窄行」的
/// 后果是一条**会被看见的拒绝**，不是一次静默超权。
const ROWS: &[(&str, &[&str], VisScope)] = &[
    (
        PRINCIPAL_MAIN,
        &[
            Capability::JudgeIntent.name(),
            Capability::Reply.name(),
            CAP_DEFINE_WORK,
            CAP_EXTERNAL_EXECUTION,
            CAP_ASSIGN_WORK,
        ],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_AUTONOMOUS,
        &[CAP_DEFINE_WORK],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_REFLEX,
        &[Capability::JudgeIntent.name()],
        VisScope::ThreadPrivate,
    ),
    (
        PRINCIPAL_PEER,
        &[CAP_EXTERNAL_EXECUTION],
        VisScope::ThreadPrivate,
    ),
    // 组织形态三角色（S13 §3，）：市场派活（公共域）/ 仲裁盖章（公共域）/ 守门判意图（私有）。
    // 三行都是**单能力独持**——与自主 / 判定 / 对等三行同一条纪律：角色出现必须自带窄行，
    // 窄行是唯一入口（D-099）。arbiter 持 `assert.verification` 不碰 C15（不持
    // `define.work` / `produce.artifact`，构造期双持检查见 `governance`）。
    (PRINCIPAL_MARKET, &[CAP_ASSIGN_WORK], VisScope::Public),
    (
        PRINCIPAL_ARBITER,
        &[CAP_ASSERT_VERIFICATION],
        VisScope::Public,
    ),
    (
        PRINCIPAL_GATEKEEPER,
        &[Capability::JudgeIntent.name()],
        VisScope::ThreadPrivate,
    ),
    // S02/S03 三角色行（行与执行路径同批）：工人产生产物（树侧委派收束）、
    // 规划者立任务、验证者独立判定。三行都是**单能力独持**——与自主 / 判定 / 对等 /
    // 市场 / 仲裁 / 守门六行同一条纪律：角色出现必须自带窄行，窄行是唯一入口
    // （D-099）。planner 持 `define.work` 不碰 C15（不持 `assert.verification`）；verifier
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
    // 三族工具主体行（D-069 §决策 2 工具响应主体， 落地）：工具的权威动作是
    // **产生产物**（`produce.artifact`）——工具响应的正文就是一条 artifact。与
    // `session/worker` / `session/peer-b` 同款独持（窄的方向相同而角色不同：worker 是
    // 树侧委派落点、peer 是图侧协作方、tool 是**能力**发言方）。今天写侧闸按会话
    // 主体判，这三行是**声明性登记**，为 之后 `plugin::attributed` 按 kind=Tool
    // 补 principal 铺好底。缺这一行不会红——表里没它 ⇒ `production_matrix` 没有它的行
    // ⇒ `can_produce_artifact(vdfs/write)` fail-closed（D-099）；
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

static MATRIX: OnceLock<EffectiveMatrix> = OnceLock::new();

/// **运行期绑定登记**（[D-100](../../../../docs/003-design/008-能力注册表与能力坐标系.md#d-100-绑定即登记能力随会话创建而来与会话同寿可见域不登记 "DE-008-能力注册表与能力坐标系")：绑定即登记）：
/// 主体 ⇒ 各**会话**为它声明的能力行。
///
/// 与 [`ROWS`] 的分工：表是**部署事实**（常量、构造期定稿），这里是**运行期身份事实**
/// ——哪个 agent 身份真的被哪个会话绑着。查询侧合流（[`EffectiveMatrix`]）：
/// **登记过的绑定行就是有效行**，未登记才回退部署行；委派子会话登记的是
/// 父行 ∩ 自己的声明行（[`bind_inheriting`]）。可见域**不**登记
/// （[`EffectiveMatrix::can_see`] 只认部署表）：能力可以随绑定来，
/// 「看见别人的 shared」只能来自部署登记。
///
/// **登记与会话同寿**：
/// 每条登记都带**登记者**（会话 id），会话删除时随 [`unbind_session`] 整册回收。
/// 同一会话重复登记只替换自己那一条（幂等）；同一主体被多个存活会话登记时取
/// **最近一次**——收束闸与委派上限的读法不变，变的是行不再比会话活得久。
type BoundRows = HashMap<String, Vec<Binding>>;

/// 一条绑定：**登记者**（会话 id）与它声明的能力行。
struct Binding {
    holder: String,
    grants: Vec<Capability>,
}

static BOUND: OnceLock<RwLock<BoundRows>> = OnceLock::new();

fn bound_rows() -> &'static RwLock<BoundRows> {
    BOUND.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 取绑定面写锁；锁中毒只告警——**登记失败 = 不登记**，方向与 D-099 一致。
fn write_bound() -> Option<std::sync::RwLockWriteGuard<'static, BoundRows>> {
    match bound_rows().write() {
        Ok(map) => Some(map),
        Err(why) => {
            crate::plugin_warn!(
                "authz",
                "[governance] 绑定登记写入失败（fail-closed）：{why}"
            );
            None
        }
    }
}

/// 清掉某个会话登记过的**全部**行（身份改写时先撤旧行，回收时整册删除）。
fn clear_session_rows(map: &mut BoundRows, holder: &str) {
    map.retain(|_, entries| {
        entries.retain(|binding| binding.holder != holder);
        !entries.is_empty()
    });
}

/// **根会话绑定**（[D-100]）：`Scope::Root` 没有父主体，只有一条来源——
/// 会话面缺省行（[`PRINCIPAL_MAIN`]）。
///
/// 已部署的主体（[`ROWS`] 里有自己的行）**原样用自己的行**：根不是委派，没有父行
/// 可作为上限。拿会话面去「夹」它会把主智能体刻意不持的能力从窄行身上削掉——
/// 例如 `session/verifier` / `session/arbiter` 的 `assert.verification`
/// （C15：主智能体不自验），父窄子宽的交集在这里不适用。
///
/// 未部署的主体（`--agent <id>` 选定的会话身份）才登记缺省行——这是 D-100
/// 「能力随绑定来」的入口。
///
/// `session_id` 是**登记者**：登记行随这个会话的创建而来、随它的删除而去
/// （[`unbind_session`]）。同一会话改写身份时旧行先被撤掉——否则旧身份会挂着
/// 一条谁也不认领的登记，直到会话结束。
pub(crate) fn bind_root(session_id: &str, principal: &str) {
    if session_id.is_empty() {
        return;
    }
    let matrix = production_matrix();
    // 已部署主体用自己的行；主智能体本身也无需登记。
    let grants = if principal == PRINCIPAL_MAIN || matrix.is_deployed(principal) {
        None
    } else {
        Some(matrix.grants_of(PRINCIPAL_MAIN))
    };
    let Some(mut map) = write_bound() else { return };
    clear_session_rows(&mut map, session_id);
    if let Some(grants) = grants {
        insert_binding(&mut map, session_id, principal, grants);
    }
}

/// **委派绑定**（[D-100]）：`Scope::Child(parent)` 按
/// **父能力 ∩ 子主体的显式声明**登记——委派不放大权威。
///
/// - `session_id`：登记者（会话 id）——行随会话创建而来、随删除而去；
/// - `principal`：会话主体（[`request_principal`] 的产出）；
/// - `from`：父主体（`Scope::Child(parent)` 的 `parent`）。
///
/// 三条边界：
///
/// 1. **交集不是并集**：子主体已在 [`ROWS`] 里 ⇒ 它的部署行是**显式声明**，
///    有效行 = 父行 ∩ 声明行；未部署 ⇒ 没有声明，整个有效行来自父。父窄子宽时
///    子只保留父也持有的能力，父宽子窄时子仍只有自己声明的那些——两个方向都不
///    放大权威；
/// 2. **`from` 无行时先给 `from` 登记会话面缺省**——委派上限的两侧都要在册
///    （`turn_runner::within_delegation_ceiling` 判父行），否则子会话因父主体查不到行
///    而收束不了；
/// 3. **登记失败 = 不登记**（fail-closed 的方向）：锁中毒只告警跳过，
///    未登记的主体照旧每个谓词为假。
///
/// 幂等：同一会话重复绑定不重复写、不重复告警；先撤本会话旧行，再登记新行。
pub(crate) fn bind_inheriting(session_id: &str, principal: &str, from: &str) {
    bind_inheriting_in(production_matrix(), session_id, principal, from);
}

/// [`bind_inheriting`] 的**取表口显式化**：判定用哪张表由调用方给，登记仍写
/// [`BOUND`]。生产只有一个调用点（[`production_matrix`]）；显式参数让交集语义能在
/// **自造小表**上验证，而不必改动生产表或共享的绑定面。
///
/// 取数一律在写锁**之前**完成：`grants_of` / `bound_row_excluding` 自己会读
/// [`BOUND`]，持写锁再调用就是自锁。
fn bind_inheriting_in(matrix: &EffectiveMatrix, session_id: &str, principal: &str, from: &str) {
    if session_id.is_empty() {
        return;
    }
    // 父侧“有没有行”要看**除本会话之外**的登记：本会话上一轮的辅助行先被排除，
    // 否则「先撤旧行再登记」会把父行误判成已存在而漏登记。
    let parent_present =
        matrix.is_deployed(from) || matrix.bound_row_excluding(from, session_id).is_some();
    let parent = if parent_present {
        matrix.effective_grants_excluding(from, session_id)
    } else {
        matrix.grants_of(PRINCIPAL_MAIN)
    };
    let row = inherited_grants(matrix, principal, &parent);
    let Some(mut map) = write_bound() else { return };
    clear_session_rows(&mut map, session_id);
    if principal == from {
        return;
    }
    if !parent_present {
        insert_binding(&mut map, session_id, from, parent);
    }
    insert_binding(&mut map, session_id, principal, row);
}

/// **会话结束 ⇒ 登记行回收**（[]）：一次删掉该会话登记过的全部行。
///
/// 与 [`bind_root`] / [`bind_inheriting`] 的登记者是同一个键——会话 id 因此是
/// 绑定面的**生命周期单位**，进程级只增不减只属于「会话还活着」的那些行。
pub(crate) fn unbind_session(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let Some(mut map) = write_bound() else { return };
    let mut removed = 0usize;
    map.retain(|_, entries| {
        let before = entries.len();
        entries.retain(|binding| binding.holder != session_id);
        removed += before - entries.len();
        !entries.is_empty()
    });
    if removed > 0 {
        crate::plugin_info!(
            "authz",
            "[governance] 会话 {session_id} 结束，回收 {removed} 行绑定登记"
        );
    }
}

/// 委派交集的**唯一判据**：父行 ∩ 子主体的部署行；子未部署 ⇒ 父行原样继承。
///
/// 空交集是合法结果（双方都在册、只是没有共同能力）——它登记为**空行**，
/// 不是「没登记」：后者会退回部署行，正好把「父不持 ⇒ 子也不持」翻成一次超权。
fn inherited_grants(
    matrix: &EffectiveMatrix,
    principal: &str,
    parent: &[Capability],
) -> Vec<Capability> {
    let declared = matrix.deployed.grants_of(principal);
    if declared.is_empty() {
        return parent.to_vec();
    }
    parent
        .iter()
        .copied()
        .filter(|cap| declared.contains(cap))
        .collect()
}

/// 登记一行绑定（幂等：同一会话同一行则不动；同会话换行则替换并移到队尾）。
///
/// **空行也是登记**：委派交集为空时，登记 `[]` 表达「这个主体什么都不许」。
/// 若把空行当作「不登记」跳过，查询面会退回部署行——父不持、子却照旧持有，
/// 正好是「委派不放大权威」的反面。
fn insert_binding(map: &mut BoundRows, holder: &str, principal: &str, grants: Vec<Capability>) {
    let entries = map.entry(principal.to_string()).or_default();
    match entries.iter().position(|binding| binding.holder == holder) {
        Some(i) if entries[i].grants == grants => return,
        Some(i) => {
            let mut binding = entries.remove(i);
            binding.grants = grants;
            entries.push(binding);
        }
        None => entries.push(Binding {
            holder: holder.to_string(),
            grants,
        }),
    }
    let granted = entries
        .last()
        .map(|binding| binding.grants.len())
        .unwrap_or(0);
    crate::plugin_info!(
        "authz",
        "[governance] 绑定即登记：{principal} ⇒ {granted} 项能力（登记者 {holder}；能力随绑定来，可见域不登记）"
    );
}

/// 本机会话的**主体派生**（部署事实 → [`PRINCIPAL_MAIN`] 同族的取值点）：
/// 会话选定了哪个 agent，这轮所有断言就是哪个主体；未选 ⇒ 主智能体。
///
/// 判据取自 core 的命名空间登记表 [`principal_namespace`]
/// （[D-069 §决策 3](../../../../docs/003-design/005-协议规范.md#d-069-principal-三字段kind-与-id-分家id-是可路由地址 "DE-005-协议规范")）——
/// 「什么地址算哪一族」只定义一次，可见域判据（`view::is_agent_id`）与身份派生
/// 共用登记表，两处各写一个字面量迟早漂移成「判的是 A、写的是 B」。
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
/// 为什么住 core 而不留会话插件（[D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 1 组）：
/// 它是对 core 既有键（`AGENT_ID`）的**纯派生函数**，不属于任何一个切片；会话驱动器
/// 切成独立插件后（[plan/14 步 6](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")），
/// 读侧（`session/stats`）与写侧隔着 E-009 的插件墙两头都要用，留在任一侧都会变成
/// 另一条反向引用。
pub(crate) fn request_principal(ctx: &dyn crate::symbio_core::PluginInvokeRequest) -> String {
    use crate::symbio_core::PluginInvokeRequestExt;
    principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref())
}

/// 本机授权矩阵——**该主体当次判定的唯一取表口**（行的形状不变：`(主体, 能力名, 可见域)`，
/// 见本模块文档「本表会怎么长」）。懒构造一次；构造失败 ⇒ 空矩阵 = 谁都不许，并告警。
///
/// 判定口径两条，合成才是「现在准不准」（[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范") +
/// [D-100](../../../../docs/003-design/008-能力注册表与能力坐标系.md#d-100-绑定即登记能力随会话创建而来与会话同寿可见域不登记 "DE-008-能力注册表与能力坐标系")）：
///
/// - **能力面**（`grants_*` / `can_write*` / `can_reply` / `can_produce_artifact`）=
///   部署表，绑定登记存在时以**绑定行为有效行**（未登记才回退
///   部署行）——根会话未部署时登记会话面缺省，委派子会话登记父行 ∩ 自己的声明行，
///   两者都没有 ⇒ 每个谓词为假 ⇒ **fail-closed**：少一行是一条**拒绝**，不是一次
///   静默超权；
/// - **可见域面**（`sees_of` / `can_see`）只认**部署表**：「谁能看见谁的 shared」是
///   部署登记的事实，不随会话绑定而来——否则任何绑定 agent 都会变成矩阵成员、
///   读到他人 shared（t76 的反判据）。
///
/// 写侧收束、读侧可见域与外部执行闸门的授权判据都取这一份，保证「表只有一张」——
/// 各判各的就会漂移成「闸放行、表说没有」。
pub(crate) fn production_matrix() -> &'static EffectiveMatrix {
    MATRIX.get_or_init(|| {
        let deployed = PermissionMatrix::from_names(ROWS).unwrap_or_else(|why| {
            crate::plugin_warn!(
                "authz",
                "[governance] 授权表构造失败，按 fail-closed 降级为空矩阵（{}）：{}",
                ROWS.len(),
                why
            );
            PermissionMatrix::from_names(&[]).expect("空授权表无主体可校验，不可能违反成对性")
        });
        EffectiveMatrix { deployed }
    })
}

/// 取表口的查询面（[`production_matrix`] 的返回类型）：部署表 + 运行期绑定登记的合流。
///
/// 住本模块而不是 `governance`：合流口径是**部署事实怎么看待运行期身份**
/// （绑定行替换部署行、但不带可见域），那是这张表的裁决，不是判定机制的裁决；
/// `governance` 的 [`PermissionMatrix`] 因此保持「常量表 + fail-closed 谓词」原样。
///
/// ## 两个读数的名字（[D-100](../../../../docs/003-design/008-能力注册表与能力坐标系.md#d-100-绑定即登记能力随会话创建而来与会话同寿可见域不登记 "DE-008-能力注册表与能力坐标系")「淘汰面为什么
/// 只认部署表」一节，口径只在那一节写，这里不重述理由）
///
/// | 名字 | 成员 | 回答的问题 |
/// |---|---|---|
/// | **合流面** | `can_write` / `can_write_name` / `can_reply` / `can_produce_artifact` / `grants_of` / `grant_names_of` | 「**现在**这个主体能不能做这件事」（绑定行替换部署行，未登记才回退） |
/// | **部署事实面** | `deployed_can_write_name` / `is_deployed` | 「这台机器**部署过**谁、给它配过什么」（只认 [`ROWS`]） |
///
/// 调用点不点名本类型（类型推导取方法）——收窄/扩面都在这里一处发生。
pub(crate) struct EffectiveMatrix {
    deployed: PermissionMatrix,
}

impl EffectiveMatrix {
    /// 绑定登记半边（[`BOUND`]）：`Some(行)` = 该主体登记过（**空行也算登记**）；
    /// `None` = 未登记。两者在合流面的后果不同：前者以登记行为准，后者回退部署行。
    ///
    /// 同一主体被多个存活会话登记时取**最近一次**（队尾）——这是「身份是主体、
    /// 绑定是会话事实」的唯一冲突口径。
    fn bound_row(&self, principal: &str) -> Option<Vec<Capability>> {
        let map = bound_rows().read().ok()?;
        map.get(principal)
            .and_then(|entries| entries.last())
            .map(|binding| binding.grants.clone())
    }

    /// 同 [`Self::bound_row`]，但**排除某个会话**的登记——委派登记先撤本会话旧行，
    /// 再据此判断父侧是否真的另有行（否则辅助行会被误判成外部登记而漏写）。
    fn bound_row_excluding(&self, principal: &str, holder: &str) -> Option<Vec<Capability>> {
        let map = bound_rows().read().ok()?;
        map.get(principal)?
            .iter()
            .rev()
            .find(|binding| binding.holder != holder)
            .map(|binding| binding.grants.clone())
    }

    /// 合流后的有效能力集：**登记过的绑定行就是有效行**（含空行——空 = 什么都不许）；
    /// 未登记才读部署表。
    ///
    /// 为什么不是「部署行 ∪ 绑定行」：委派子会话的绑定行是**父行 ∩ 自己的声明行**
    /// （[`bind_inheriting`]），再与部署行取并集会把刚夹掉的能力加回来——
    /// 「委派不放大权威」当场失效。
    fn effective_grants(&self, principal: &str) -> Vec<Capability> {
        match self.bound_row(principal) {
            Some(row) => row,
            None => self.deployed.grants_of(principal).to_vec(),
        }
    }

    /// 同 [`Self::effective_grants`]，但排除某个会话的登记（[`bind_inheriting`] 的
    /// 父行取值用）。
    fn effective_grants_excluding(&self, principal: &str, holder: &str) -> Vec<Capability> {
        match self.bound_row_excluding(principal, holder) {
            Some(row) => row,
            None => self.deployed.grants_of(principal).to_vec(),
        }
    }

    /// 这个主体是**部署事实**吗（表里有行）——淘汰面的判据（只对部署主体落裁决）。
    pub(crate) fn is_deployed(&self, principal: &str) -> bool {
        !self.deployed.grants_of(principal).is_empty()
    }

    /// 写侧：主体持有的全部能力（空 = 部署表与绑定登记都没有它的行，或交集为空）。
    pub(crate) fn grants_of(&self, principal: &str) -> Vec<Capability> {
        self.effective_grants(principal)
    }

    /// 写侧：主体持有的能力**名**（授权表的书写形态，与
    /// [`PermissionMatrix::grant_names_of`] 同一套拼写；取自同一条有效行）。
    pub(crate) fn grant_names_of(&self, principal: &str) -> Vec<String> {
        match self.bound_row(principal) {
            Some(row) => row.iter().map(|cap| cap.name().to_string()).collect(),
            None => self.deployed.grant_names_of(principal),
        }
    }

    /// 写侧判定：主体是否可以权威地做 `cap`（合流面，fail-closed）。
    ///
    /// 生产读方：意图闸门（[`crate::symbio_core::actors::IntentGate::approve`]，
    /// 升格 = `define.work`）。其余调用点按能力名或收束臂问
    /// （[`Self::can_write_name`] / [`Self::can_reply`] / [`Self::can_produce_artifact`]）。
    pub(crate) fn can_write(&self, principal: &str, cap: Capability) -> bool {
        match self.bound_row(principal) {
            Some(row) => row.contains(&cap),
            None => self.deployed.can_write(principal, cap),
        }
    }

    /// 写侧判定（按能力名，合流面）。名字认不出 ⇒ `false`（fail-closed，与
    /// [`PermissionMatrix::can_write_name`] 同方向）。
    pub(crate) fn can_write_name(&self, principal: &str, name: &str) -> bool {
        match self.bound_row(principal) {
            Some(row) => row.iter().any(|cap| cap.name() == name),
            None => self.deployed.can_write_name(principal, name),
        }
    }

    /// 写侧判定（**部署事实面**）：只认 [`ROWS`]，不认绑定登记——淘汰回路的判据
    /// （`derived_facts`：未登记的运行期主体不是部署事实，不进淘汰面）。
    pub(crate) fn deployed_can_write_name(&self, principal: &str, name: &str) -> bool {
        self.deployed.can_write_name(principal, name)
    }

    /// 写侧判定（收束闸）：主体能否**收束一轮发言**——轮次位置 → 能力的映射仍只住
    /// [`Capability::Reply`]，调用点不复述；判据取有效行（含委派交集）。
    pub(crate) fn can_reply(&self, principal: &str) -> bool {
        match self.bound_row(principal) {
            Some(row) => row.contains(&Capability::Reply),
            None => self.deployed.can_reply(principal),
        }
    }

    /// 写侧判定（产物闸）：主体能否**产生产物**（[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")
    /// 的第二条收束路）；判据取有效行（含委派交集）。
    pub(crate) fn can_produce_artifact(&self, principal: &str) -> bool {
        match self.bound_row(principal) {
            Some(row) => row.contains(&Capability::ProduceArtifact),
            None => self.deployed.can_produce_artifact(principal),
        }
    }

    /// 读侧：主体的可见域（**只认部署表**——绑定登记不带可见域，见 [`Self`] 文档）。
    ///
    /// 判据的读方只有测试（成对性：行必须显式给可见域）；生产读侧一律走
    /// [`Self::can_see`] 的三元判定，不读裸可见域。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sees_of(&self, principal: &str) -> Option<VisScope> {
        self.deployed.sees_of(principal)
    }

    /// 读侧判定（**只认部署表**）：`shared` 只给部署登记的主体——绑定 agent 不因
    /// 绑定成为矩阵成员（t76 的反判据）。
    pub(crate) fn can_see(&self, viewer: &str, owner: &str, scope: VisScope) -> bool {
        self.deployed.can_see(viewer, owner, scope)
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/authz/mod.rs::production_matrix: 穷尽性不变量：上游已穷举 / 已校验，走到 else 说明本文件的判据漏了一个分支，属代码缺陷。
