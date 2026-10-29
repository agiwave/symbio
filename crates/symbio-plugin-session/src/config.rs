//! 会话插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么在插件里，而不是 core
//!
//! 它不住在 core 的 `chat` 模块（`crates/symbio-core/src/chat/`），不与那里的
//! `session_chat` / `chat_node` 并列。但两者性质不同：
//!
//! - **协议 schema**（那两个）是**跨插件契约**——agent / local / model 都按同一份
//!   定义拼 WS 帧与 payload，改了会同时影响多方，因此归核心；
//! - **本文件是配置**：全仓引用**只在 `crates/symbio-plugin-session/` 内**，前端与 CLI 零镜像，
//!   `lib.rs` 也不导出。它描述的是「本插件自己的旋钮」，不是任何跨插件接口。
//!
//! 判据与 `ChatSession` 契约下沉时同一条（见 `chat_session.rs` 模块文档）：
//! **核心架构只保留跨插件共享的抽象，不承载单一模块的内部定义。**
//! 与 `work` / `agent` 插件的 `config.rs` 也因此回到同一形态——每个插件把
//! 「自己的配置」放在自己目录下，配置的定义与它的拥有者不分离。
//!
//! ## 落盘契约不变
//!
//! 搬迁不改 serde：字段名仍是 `PLUGIN.yml` 里的键，`#[serde(default)]` 仍在，
//! 未知键仍被静默忽略。存量配置文件**无需迁移**。

use serde::{Deserialize, Serialize};
use symbio_core::ApprovalGrants;

// 建树策略的形状与 flat 折叠住 core `node_build`（[D-082](../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表")
// 裁决三，[plan/14 步 6](../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 切片批）：写侧构造
// 词表被留守与驱动器两个插件共用，隔着 E-009 的插件墙只能住 core；配置面在这里
// **续一发短路径**——`plugin.rs` / `state.rs` / `tool_dispatch.rs` 等既有引用不经改动。
pub use symbio_core::node_build::PresentationTree;

/// 会话配置 —— 本插件的旋钮（字段真源）。
///
/// ## 存储目录
///
/// Session 存储目录**不是**配置项，而是本插件**自己的目录**（装配期由父插件经
/// `PLUGIN_DIR` 告知）：`<本插件目录>`。
/// 这样 session 存储始终跟随本实例所在的作用域，顶层时是系统级会话、子智能体下
/// 是子树会话，无需任何全局查找。
///
/// ## 已移除字段
///
/// - `storage_dir`：存储根由父插件经 `PLUGIN_DIR` 告知、不可由配置改路径，
///   留着只会让人误以为可改；
/// - `session_id`：全仓零消费者、零赋值。配置本身即按会话目录存放（id 由目录名决定），
///   再在内容里存一份 id 属自指冗余。
/// - `store_kind`（及 `StoreKind` 枚举）：在 `file` / `sqlite` / `memory`
///   三个后端间选型。sqlite 是「可配置但没人能配置」——前端零引用、默认恒为 `file`、
///   不支持子会话清单，且仍需一个磁盘目录放压缩存档；memory 表达的不是「另一种存储」
///   而是「要不要持久化」。两者都不是同一件事的第二种实现，选型因此下沉到构造点
///   （`store::SessionStore::{new, ephemeral}`），配置面少一个假开关。
///
/// 旧配置里残留的上述键会被 serde 静默忽略（本结构未开 `deny_unknown_fields`），
/// 无需数据迁移。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 最大保存会话轮数（每一轮以一个 User 消息开始）
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
    /// 自动压缩（上下文 Token 用量达到有效上限 70% 时触发 LLM 语义快照）
    #[serde(default = "default_auto_compress")]
    pub auto_compress: bool,
    /// 上下文会话轮数限制（0 表示不限制，每一轮以一个 User 消息开始）
    #[serde(default = "default_context_messages")]
    pub context_messages: usize,
    /// 写入期工具链裁剪：是否在**落库时**物理删除 `context_messages` 轮之前的
    /// Tool / ToolCall / Reasoning 节点及其存档文件。
    ///
    /// 这是「存储保留事实、压缩只换基」架构原则的**唯一例外**：存在理由仅是控制
    /// 工具密集型会话的磁盘与节点树体积，本开关不影响发给模型的上下文大小
    /// （请求体是事实网格的纯投影，见 [D-078](README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "crates/symbio-plugin-session/src/README")）。
    ///
    /// - `true`（默认）：保持历史行为，超出窗口的工具链在存储层被物理删除，
    ///   前端节点树同样看不到这些历史工具调用；
    /// - `false`：存储严格保留完整原文，工具链裁剪完全交给请求视图层，
    ///   UI 可回看全部历史；此时存储层仅剩 `max_messages` 的 FIFO 轮次淘汰。
    #[serde(default = "default_prune_tool_history")]
    pub prune_tool_history: bool,
    /// 会话记忆（`<会话目录>/MEMORY.md`）的**写入**字节上限。
    ///
    /// 与 `providers/memory` 的两道闸门口径一致：超限**拒绝**（不截断、不部分
    /// 写入）——静默丢内容是最坏的一类失败，拒绝则把判断权交回模型。
    #[serde(default = "default_memory_max_bytes")]
    pub memory_max_bytes: usize,
    /// 会话记忆每轮**注入**系统提示词的字节上限；超出部分截断并告知地址。
    ///
    /// 与写入上限是**两道独立闸门**：记忆文件可以比注入预算大，超出部分靠模型按
    /// 地址 `vdfs_read` 读取。两者取值不同才有意义。
    #[serde(default = "default_memory_inject_max_bytes")]
    pub memory_inject_max_bytes: usize,
    /// 补充整合总开关（J2 平凡值：`false` ⇒ 完全退回"一条消息 = 一轮"）。
    ///
    /// ## 它控制什么
    ///
    /// 用户在会话运行中连续发多条消息时，把队列里的条目**抽干并合并成一条**
    /// 用户消息，作为**同一轮**的输入折进当前工作，而不是一条消息占一轮。
    ///
    /// 两个抽干点（见 `transcript/inbox.rs::drain_inbox_once` 与
    /// `chat_loop.rs` 的轮边界）都受本开关管辖——关掉它，两个点一起失效。
    #[serde(default = "default_supplements_enabled")]
    pub supplements_enabled: bool,
    /// 单次抽干的条目数上限；超出者**留队**，等下一个抽干点再抽。
    ///
    /// ## 为什么必须有上界
    ///
    /// 合并是"把 n 条拼成一条"。没有上界时，一次刷屏（或上游批量写入）会把
    /// 任意多条消息拼成一条巨型用户消息——那不是"整体处理"，那是把上下文一次
    /// 打爆。上界是**护栏**，不是策略：它只决定"这一批最多几条"，不决定合并与否。
    #[serde(default = "default_supplements_max_per_drain")]
    pub supplements_max_per_drain: usize,
    /// 轮首判决总开关（**J2 平凡值：`false`**）。
    ///
    /// ## 它控制什么
    ///
    /// 每一轮开始时是否经容器 `route` 请 `classify` 判一次「直接回答还是派活」：
    /// - `true`：判一次。`Answered` ⇒ 本轮不进工具循环（收尾）；
    ///   `Escalate` ⇒ 照旧进工具循环；
    /// - `false`：**不调用**，全部输入直接进工具循环——与未挂载 `classify` 时逐字一致。
    ///
    /// ## 出厂默认为什么是 `true`
    ///
    /// 判决本身只输出枚举，面向用户的那句话归 `compose`；在 `compose` 还是平凡实现
    /// （恒空串）时打开本开关，「你好」会变成**沉默**——那不是"少说一句"，那是回归。
    /// S2 因此只交付机制、默认关闭，并在字段文档里写下「`compose` 落地的那一批把默认
    /// 翻成 `true`」。S3 就是那一批：`compose` 的两条产线（模板 / 生成）都已落地，
    /// 判决为 `Answered` 时**必然有话说**（最差也是变体兜底），沉默的前提不复存在。
    ///
    /// 这与「一批一件事、每批行为可独立回退」是同一条纪律：
    /// **让功能生效的那一批，负责它带来的全部影响**。
    ///
    /// ## 为什么它在 session 的配置面，而「用不用规则表」不在
    ///
    /// 本开关问的是「**调用方**要不要请判决」，是调用方的事；而「判决内部用不用规则表」
    /// 是插件自己的策略，归 `classify` 自己的 `PLUGIN.yml`（`ClassifyConfig::rule_shortcut`）。
    /// 把后者也塞进这里，等于让调用方为一段它看不见的策略维护一个开关。
    #[serde(default = "default_classify_enabled")]
    pub classify_enabled: bool,
    /// 对话面措辞总开关（**J2 平凡值：`false`**）。
    ///
    /// ## 它控制什么
    ///
    /// 判决为 `Answered` / `Escalate` 时是否经容器 `route` 请 `compose` 说一句话：
    /// - `true`：说。`Answered` 的答话进请求包（它就是这一轮的答复）；
    ///   `Escalate` 的首响**不进**请求包（界面开场白，见 `chat_loop/compose.rs`）；
    /// - `false`：**不调用**。`Answered` 于是拿不到措辞 ⇒ **降级进工具循环**
    ///   （不沉默）；`Escalate` 没有首响。与未挂载 `compose` 时逐字一致。
    ///
    /// ## 为什么出厂默认就是 `true`
    ///
    /// 与 `classify_enabled` 相反：本开关**不需要**等谁落地。关掉它不会让任何东西
    /// 变沉默——`Answered` 的降级方向是"照旧进工具循环"，那是引入判决之前的行为，
    /// 完整可用。因此打开它是纯粹的增强，没有"先交机制再生效"的两段式需要。
    ///
    /// ## 为什么它在 session 的配置面
    ///
    /// 与 `classify_enabled` 同一条：问的是「**调用方**要不要请措辞」。措辞内部
    /// 用模板还是用模型、用哪个提示词，都是 `compose` 自己的策略，归它自己的配置面。
    #[serde(default = "default_compose_enabled")]
    pub compose_enabled: bool,
    /// 对话面开场白（`Escalate` 首响）总开关（**出厂默认：`false`**）。
    ///
    /// ## 它控制什么
    ///
    /// 判决为 `Escalate`（要派活）时，是否在干活前先写一句根级对话面开场白
    /// （"我先看一下" / "好，我来处理"，见 `compose/templates.rs`）。
    ///
    /// - `true`：本轮干活前说一句开场白（与引入本开关之前逐字一致）；
    /// - `false`：**不写开场白**，本轮照旧进工具循环。
    ///
    /// ## 为什么出厂关
    ///
    /// 那句开场白的历史职责是「点发送后立刻给用户一个动静」，而机制化前端已用
    /// Typing 占位 + 节点自身运行态覆盖了这个需求；留着它只会与执行过程信息抢视线。
    /// 真实答复（`Answered`）不走本开关，因此关掉它不会让任何**答复**消失。
    ///
    /// ## 为什么它是独立的一格
    ///
    /// 它与 `compose_enabled` 不合并：后者关掉会连真实答复一起没了（`Answered` 降级
    /// 进工具循环），而本开关只削掉派活前的那句旁白。判据见 `ChatOrchestrator` 的
    /// 两级独立开关说明。
    #[serde(default = "default_opening_enabled")]
    pub opening_enabled: bool,
    /// 中途汇报总开关（**J2 平凡值：`false`**）。
    ///
    /// ## 它控制什么
    ///
    /// 长任务进行中，助手是否在**轮边界**主动说一句进度（`Verdict::Report` ⇒
    /// `compose` 从运行现状组织一句话 ⇒ 一条根级对话面节点）。判定与执行见
    /// `chat_loop/progress.rs`。
    ///
    /// - `true`：静默超过 [`Self::progress_interval_ms`]、且已跑够
    ///   [`Self::progress_min_rounds`] 轮时，汇报一次（每轮至多
    ///   [`Self::progress_max_per_turn`] 次）；
    /// - `false`：**不判定**，一次都不汇报——与引入汇报之前逐字一致。
    ///
    /// ## 为什么出厂默认为 `false`
    ///
    /// 机制化前端已经按节点自身呈现「在跑什么 / 跑了多久」（工具子骨架的运行态 +
    /// 已运行秒数）与「还没出声」（Typing 占位），轮边界那句"已完成 N 轮"与执行过程
    /// 信息重复、反而干扰视线。它的历史职责（让用户区分"在干活"与"卡死了"）已由
    /// 前端承担。代价被三个上界钉住（间隔 / 最少轮次 / 每轮次数），要恢复打断式
    /// 汇报把本开关打开即可——判定与措辞的代码路径都在。
    ///
    /// ## 为什么它在 session 的配置面
    ///
    /// 「什么时候该打断用户」是**编排**的判断（它要读会话状态：静默时长、轮次、
    /// 已汇报次数），不是措辞插件的策略——`compose` 只负责把给它的现状说成人话。
    #[serde(default = "default_progress_enabled")]
    pub progress_enabled: bool,
    /// 汇报的**静默阈值**（毫秒）：距对话线上最近一次动静（用户发言 / 助手说话）
    /// 超过它，才认为"用户等太久了，该说一句"。
    ///
    /// 它同时是措辞里那句"已经 N 分钟了"的来源——**同一个数**，不另算一份。
    #[serde(default = "default_progress_interval_ms")]
    pub progress_interval_ms: u64,
    /// 汇报的**最少轮次**：已完成的工具轮次达到它才有"进展"可报。
    ///
    /// 第一轮就跑完的任务不该被打断——那时用户刚说完话，一句"我已经跑了一轮"
    /// 是噪声。
    #[serde(default = "default_progress_min_rounds")]
    pub progress_min_rounds: usize,
    /// **每轮**汇报次数上限（护栏）：`0` ⇒ 一次都不汇报（与 `progress_enabled`
    /// 同效，但语义不同——前者是"这个特性关掉"，后者是"上界为零"）。
    ///
    /// 上界的理由与 `supplements_max_per_drain` 同一条：判定是"该不该说"，
    /// 上界是"最多说几次"。没有它，一个长时间运行的任务会按间隔反复刷屏。
    #[serde(default = "default_progress_max_per_turn")]
    pub progress_max_per_turn: u32,
    /// 「欲」升格为任务的总开关（**E3 平凡值：`false`**，[02 §2.3](../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")）。
    ///
    /// ## 它控制什么
    ///
    /// 心跳 tick 表达出来的意图（`conation.expressed`）要不要经 `IntentGate` 升格成
    /// 长目标任务（`task.opened`，`budget_ms = 86400000`）：
    /// - `true`：闸门按 `ConationPolicy` 评估（目标过宽仍拒），通过即开格；
    /// - `false`：`IntentGate::approve` 恒 `Err("conation disabled")` ⇒ **一条自主
    ///   任务都不产生**，而「欲」照样入格、照样可被读——退化为纯响应式且完整运行。
    ///
    /// ## 与心跳开关是**两层**，不合并
    ///
    /// 触发（`HeartbeatConfig::enabled`，按会话、藏在 `Session.metadata`）问的是
    /// 「系统要不要定时做事」；本开关（按插件、藏在 `PLUGIN.yml`）问的是「做出来的事
    /// 要不要升格为长期任务」。两者的平凡值都关，但关掉的是**不同的东西**——合成一个
    /// 就没法只关其中一层（S12 §4「两层开关独立」）。
    ///
    /// 为什么放 session 的配置面：它是**治理策略**（授权 / 预算 / 审计的入口），
    /// 不是某个插件的内部参数，归编排侧的旋钮面（与 `classify_enabled` 同一条）。
    ///
    /// ## 开着之后还有一道长度闸
    ///
    /// 闸门按 `ConationPolicy::default().max_goal_len`（**200 字节**）再判一次
    /// 「目标过宽 ⇒ 拒」。心跳拿 `HeartbeatConfig::prompt` 本身当目标，中文提示词
    /// 3 字节一个字 ⇒ 约 66 字之后就会每趟被 `goal too broad` 拒掉（拒绝会记日志、
    /// `conation.expressed` 照样入格，但永远开不出长任务）。要开长目标，提示词就得
    /// 短于这个上界。
    #[serde(default = "default_conation_enabled")]
    pub conation_enabled: bool,
    /// 技能编译的总开关（**S11 §4 平凡值：`false`**，[roadmap/S11 §4](../../../docs/003-design/041-技能编译与自我改进.md "DE-041-技能编译与自我改进")）。
    ///
    /// ## 它控制什么
    ///
    /// 轮末收束时，本轮的**成功轨迹**（`chat.assistant.final` 收束格）要不要被固化成一条
    /// `memory.encoded{tag:"skill"}`——S11 §4 的平凡值 `projection = recall` 说的
    /// 正是「不编译，只检索」：
    /// - `true`：`skill_events::compile` 在收束时写技能事件（同文去重，同一句 trigger
    ///   至多编译一次），本轮的用户发言与助手回答一起被固化下来；
    /// - `false`：**零技能事件** ⇒ 读侧 `skill_events::route` 逐条跳过，置信度闸与
    ///   反自动化回退整条链路原地待命、不产生任何事实——与 [`conation_enabled`]
    ///   同一条「关掉后系统退化为纯响应式且仍完整运行」。
    ///
    /// ## 它**不**等于"开了就更快"
    ///
    /// 快路（`SkillRoute::SkillFastPath`）今天只作用在**提示词**上：低置信的技能
    /// 会被摘出本轮召回视图（回退）。「真正跳过模型、跑出 S11 §6.2 的命中 P99 比
    /// 未命中快 ≥ 3×」是执行档位与埋点那半边的事，不在本开关的管辖内。
    #[serde(default = "default_skill_compile_enabled")]
    pub skill_compile_enabled: bool,
    /// 技能快路的**执行半边**（[roadmap/S11 §4](../../../docs/003-design/041-技能编译与自我改进.md "DE-041-技能编译与自我改进")
    /// 参数表里的 `actor.pattern`：完整值 `decider`、**平凡值 `reasoner`**，默认 `false`）。
    ///
    /// ⚠️ **与 core 的 `ActorSpec.pattern` 同名不同物**（[D-079](../../../docs/003-design/005-协议规范.md#d-079-同名两物core-的-actorspecpattern-是主体分类s11-4-的-actorpattern-是开关参数前者不该做分派点 "DE-005-协议规范")）：
    /// 那个字段是**主体分类**（由授权谓词派生：持 `reply.*` ⇒ `Reasoner`），从不做分派；
    /// 这里是**逐轮开关**（命中时跳不跳模型）。两者名字撞了，把后者当成分派点会立刻
    /// 把本开关关死——`session/main` 派生 `Reasoner`，接上去快路永不命中。
    ///
    /// ## 它控制什么
    ///
    /// [`skill_compile_enabled`] 让技能**存在**、`skill_events::route` 让低置信技能**不进
    /// 提示词**；本开关决定命中时**要不要跳过模型**：
    /// - `true`：本轮发言**逐字命中**某条已编译技能（且该技能过置信度闸）⇒ 本轮以
    ///   **技能正文**收束，**一次模型调用都不发生**（S11 §2「命中技能后走 `budget_ms`
    ///   更小的 ActorSpec」、§6.2「命中后 `cost_ms` 显著低于未命中路径」）；
    /// - `false`：**永远走完整推理**——退化成 S03（S11 §4 平凡值）。
    ///
    /// ## 与 [`skill_compile_enabled`] 为什么是两个开关
    ///
    /// S11 §4 的两行参数是独立的：`projection = skill_compile`（编不编）与
    /// `actor.pattern`（命中后走不走快路，即本开关）。合成一个会让「只编译、不加速」
    /// 无法表达——而那正是**观察编译质量**时最需要的形态（编译是数据，加速是行为）。
    ///
    /// ## 命中判据取**恒等**，不是"同类"
    ///
    /// 命中 = 本轮发言与技能的 `trigger` **逐字相同**（归一化 = `trim` + 与写方
    /// **同一个**截断上限）。更宽的"同类"判定是算法问题（S11 §7：架构只提供
    /// `skill_compile` 这个位置），本开关不假装它已解决——因此命中率取决于
    /// **重复发言**，不取决于语义相似。这是诚实划界，不是缺陷：后续接入匹配算法时，
    /// 改的是**数据**（`skill_events::take_match` 一处），不是结构。
    ///
    /// ## 跳过的是 v2 运行器里的那一次模型调用
    ///
    /// 判定照常发生（观测照常落格，与关着时同形），只是没人拿它去装配——
    /// 这就是它作为一个独立开关的全部含义。见 `chat_loop/inputs.rs` 的 `fast_armed`。
    #[serde(default = "default_skill_fast_path")]
    pub skill_fast_path: bool,
    // ── Q1：worker 启动判据的三条旋钮（D-047 批 B 的「独立一批」）────────
    //
    // 出厂值即判据原样，两个平凡值都是**这一层关闭**，不是「永远命中」：
    // 阈值 0 若也判长度，任何非空消息都命中 ⇒ 每条寒暄都开一个后台会话，
    // 那正是 D-047 被否决方案的第一条。
    /// 显式前缀（**带尾空格**：`/work 干活` 命中，`/worker 是什么` 不命中）。
    /// 空串 = 本层关闭（`starts_with("")` 恒真，故显式跳过）。
    #[serde(default = "default_worker_force_prefix")]
    pub worker_force_prefix: String,
    /// 关键词（子串、忽略大小写），**逗号分隔**；空串 = 空集 = 本层关闭。
    ///
    /// 刻意用逗号串而非数组：面板控件只有 `toggle / text / number / select`，
    /// 声明一个没有对应控件的数组键，就是「后端存在、产品里开不了」——
    /// `config_definition_covers_every_session_config_field` 正是防这个形态。
    #[serde(default = "default_worker_keywords")]
    pub worker_keywords: String,
    /// 主题长度阈值（字符数）。**0 = 本层关闭**。
    #[serde(default = "default_worker_min_chars")]
    pub worker_min_chars: usize,
    /// 建树策略（`grouped` / `flat`，出厂默认 `grouped` = 今天行为；见 [`PresentationTree`]）。
    ///
    /// 后端组树时决定本轮节点的 `parent_id` 挂哪，前端只按 `parent_id` 插树、
    /// **不知道这个开关存在**——这是 [] 判据的核心：切档前端零代码改动。
    /// 认不出的写法按出厂默认处理（手写 `PLUGIN.yml` 写错不该改变组树），与
    /// `SettingConfig::verbosity` 同一条读侧容错纪律。`deep` 档随子会话转播桥
    /// 改造在 149-ii 接线。
    #[serde(default = "default_presentation_tree")]
    pub presentation_tree: String,
    /// 调度容量（`readyset` 的 `cap`）：**同时**是「一轮最多派几笔活」。
    ///
    /// ## 它是 `cap` 的写方
    ///
    /// [DE-034](../../../docs/003-design/034-并发调度与租约.md "DE-034-并发调度与租约") §4 的平凡值是
    /// `cap = 1`：调度器、租约、仲裁的逻辑**一行不改**，系统退化为串行 FIFO。
    /// 这个数不是行常量、也不是调度器自己的字段——它是**会话**的旋钮，因为「同时
    /// 能推进几件事」是这台机器上这个会话的配置。
    ///
    /// `0` 与 `1` 同义（容量为零的调度器不是「更保守」，是「永不动」）：
    /// 读侧按 `cap.max(1)` 归一，不把「写错一个数」变成「系统停摆」。
    #[serde(default = "default_task_cap")]
    pub task_cap: usize,
    /// 跨会话的工具授权清单（审批卡第三档「永久放行」）：能力名 → 允许的最高风险档。
    ///
    /// ## 为什么住会话插件的全局配置
    ///
    /// 跨会话是它的定义（「永久」），所以不能进会话 metadata；而它与会话阈值
    /// （`ctx[RISK_LEVEL]`）是同一件策略的两级（按工具 / 按会话），本插件已经是
    /// 唯一注入执行阈值的地方，由同一个解析点落进 ctx 才不会出现两处口径
    /// （D-170 决策 2）。
    ///
    /// 写入口是控制路由 `session/grant`（只抬不降）；撤销就是删掉这里的一行
    /// ——没有第二套撤销动作。认不出的档位 / 空工具名在读侧逐条丢弃（fail-closed，
    /// 见 [`ApprovalGrants`]）。
    #[serde(default)]
    pub approval_grants: ApprovalGrants,
}

// ── Q1 判据的出厂值：单一真源 ─────────────────────────────────────────
//
// 出厂值落在**配置面**而非判定模块——两者都是私有模块，判定侧能 import
// 配置面、配置面反过来够不着判定侧；写成两处字面量就是必然漂移的第二份。

/// 出厂值：显式前缀，**带尾空格**——`/work 干活` 命中，`/worker 是什么` 不命中。
pub const FORCE_PREFIX: &str = symbio_core::WORKER_FORCE_PREFIX;
/// 出厂值：关键词（子串、忽略大小写）。**空集 = 本层关闭**。
pub const KEYWORDS: &[&str] = symbio_core::WORKER_KEYWORDS;
/// 出厂值：主题长度阈值（字符数）。**0 = 本层关闭**。
pub const MIN_CHARS: usize = symbio_core::WORKER_MIN_CHARS;

pub fn default_worker_force_prefix() -> String {
    FORCE_PREFIX.to_string()
}
pub fn default_worker_keywords() -> String {
    KEYWORDS.join(",")
}
pub fn default_worker_min_chars() -> usize {
    MIN_CHARS
}

/// 出厂值：调度容量（平凡值 = 串行 FIFO）。
pub const TASK_CAP: usize = symbio_core::TASK_CAP;

pub fn default_task_cap() -> usize {
    TASK_CAP
}

pub fn default_presentation_tree() -> String {
    PresentationTree::Grouped.as_str().to_string()
}

pub fn default_max_messages() -> usize {
    100
}
pub fn default_auto_compress() -> bool {
    true
}
pub fn default_context_messages() -> usize {
    6
}
pub fn default_prune_tool_history() -> bool {
    true
}
pub fn default_memory_max_bytes() -> usize {
    16 * 1024
}
pub fn default_memory_inject_max_bytes() -> usize {
    4 * 1024
}
pub fn default_supplements_enabled() -> bool {
    true
}
pub fn default_supplements_max_per_drain() -> usize {
    20
}
pub fn default_classify_enabled() -> bool {
    // S3 生效：`compose` 的两条产线已落地，`Answered` 必然有话说（最差是变体兜底），
    // 沉默的前提不复存在。S2 的"先交机制、默认关闭"到此结束——见字段文档。
    true
}
pub fn default_compose_enabled() -> bool {
    // 关掉它不会让任何东西变沉默（`Answered` 降级进工具循环），因此无需两段式。
    true
}
pub fn default_progress_enabled() -> bool {
    // 出厂**关闭**：机制化前端已按节点自身呈现「在跑什么 / 跑了多久」（工具子骨架的
    // 运行态 + 已运行秒数）与「还没出声」（Typing 占位），轮边界那句"已完成 N 轮"
    // 与执行过程信息重复、反而干扰。要恢复打断式汇报，把本开关打开即可（代码路径与
    // 判定都在）。
    false
}
/// 对话面开场白（`Escalate` 首响）总开关（**出厂默认：`false`**）。
///
/// 关掉后本轮**照旧干活**，只是不写那句开场白（"我先看一下" / "好，我来处理"）——
/// 它曾是「点发送后立刻有个动静」的唯一手段，而机制化前端的 Typing 占位已接管这个
/// 职责，那句模板旁白于是与执行过程信息重复。真实答复（`Answered`）不受本开关影响。
pub fn default_opening_enabled() -> bool {
    false
}
pub fn default_progress_interval_ms() -> u64 {
    // 60s：短于它的任务本来就快，用户不需要被打断；长于它的才值得说一句。
    60_000
}
pub fn default_progress_min_rounds() -> usize {
    // 2 轮：一轮就完事的任务，"进展"还谈不上。
    2
}
pub fn default_progress_max_per_turn() -> u32 {
    // 5 次：一个 10 分钟的任务最多被打断 5 次，间隔已由 `progress_interval_ms` 保证。
    5
}

pub fn default_conation_enabled() -> bool {
    // E3 平凡值（02 §2.3）：关掉后「欲」照样入格、照样可读，只是不升格为任务——
    // 系统退化为纯响应式且**仍完整运行**。开不开它是价值判断，不是架构选择。
    false
}

pub fn default_skill_compile_enabled() -> bool {
    // S11 §4 平凡值：编译开关 = false（不编译，只检索）。关掉后技能链路
    // （编译 → 校准 → 回退）整条退到「没有技能」的形态，而召回 / 巩固照常跑；
    // 技能从经验里学来，不是系统的前提。
    //
    // 翻向 true 的前置条件（写死， §3.1 同步）：① S11 读数落地
    // （凸度或明确替代刻度——今天只有 reliability）；② DE-016 价值验收埋点出数。
    // 两条都满足才归届时的开关批翻向；在那之前 false 是安全退路口。
    false
}

pub fn default_skill_fast_path() -> bool {
    // S11 §4 平凡值：参数 `actor.pattern = reasoner`（永远走完整推理，退化成 S03）——
    // 注意这是**参数表的取值**，不是 core 的 `ActorSpec.pattern`（D-079）。
    // 开它是**行为改变**（命中时以技能正文收束、不调模型），因此默认关：
    // 「不加速」是安全退路口，而"只编译不加速"本身是有意义的观察形态。
    false
}

impl SessionConfig {
    /// 生效的会话记忆**写入**上限（下界 1 字节，避免配成 0 后一切写入都失败却看不出原因）
    pub fn effective_memory_max_bytes(&self) -> usize {
        self.memory_max_bytes.max(1)
    }

    /// 生效的会话记忆**注入**预算（下界 1 字节，理由同上）
    pub fn effective_memory_inject_bytes(&self) -> usize {
        self.memory_inject_max_bytes.max(1)
    }

    /// 生效的建树策略：认不出的写法按出厂默认 [`PresentationTree::Grouped`] 处理。
    ///
    /// 读侧容错与 `verbosity` 同纪律：手写 `PLUGIN.yml` 写错、或写了尚未生效的档，
    /// 都不该改变组树行为。
    pub fn presentation_tree(&self) -> PresentationTree {
        match self.presentation_tree.trim() {
            "flat" => PresentationTree::Flat,
            "deep" => PresentationTree::Deep,
            _ => PresentationTree::Grouped,
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_messages: default_max_messages(),
            auto_compress: default_auto_compress(),
            context_messages: default_context_messages(),
            prune_tool_history: default_prune_tool_history(),
            memory_max_bytes: default_memory_max_bytes(),
            memory_inject_max_bytes: default_memory_inject_max_bytes(),
            supplements_enabled: default_supplements_enabled(),
            supplements_max_per_drain: default_supplements_max_per_drain(),
            classify_enabled: default_classify_enabled(),
            compose_enabled: default_compose_enabled(),
            opening_enabled: default_opening_enabled(),
            progress_enabled: default_progress_enabled(),
            progress_interval_ms: default_progress_interval_ms(),
            progress_min_rounds: default_progress_min_rounds(),
            progress_max_per_turn: default_progress_max_per_turn(),
            conation_enabled: default_conation_enabled(),
            skill_compile_enabled: default_skill_compile_enabled(),
            skill_fast_path: default_skill_fast_path(),
            worker_force_prefix: default_worker_force_prefix(),
            worker_keywords: default_worker_keywords(),
            worker_min_chars: default_worker_min_chars(),
            presentation_tree: default_presentation_tree(),
            task_cap: default_task_cap(),
            approval_grants: ApprovalGrants::default(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
