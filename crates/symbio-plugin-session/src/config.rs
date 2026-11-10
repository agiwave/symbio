//! 会话插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么在插件里，而不是 core
//!
//! 它不住在宿主内核的 `chat` 模块（`crates/symbio-kernel/src/chat/`），不与那里的
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
use symbio_sdk::capability::ApprovalGrants;

// 建树策略的形状与 flat 折叠住 core `node_build`（[D-082](../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表")
// 裁决三）：写侧构造
// 词表被留守与驱动器两个插件共用，隔着 E-009 的插件墙只能住 core；配置面在这里
// **续一发短路径**——`plugin.rs` / `state.rs` / `tool_dispatch.rs` 等既有引用不经改动。
pub use symbio_sdk::turn::node_build::PresentationTree;

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
///   而是「要不要持久化」。两者都不是同一件事的第二种实现，且 D-329 已裁决
///   无事实源的驻留方式退场——存储只有磁盘构造点 `store::SessionStore::new`。
///
/// 旧配置里残留的上述键会被 serde 静默忽略（本结构未开 `deny_unknown_fields`），
/// 无需数据迁移。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 最大保存会话轮数（每一轮以一个 User 消息开始）
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
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
    /// `progress.rs`。
    ///
    /// - `true`：静默超过 [`Self::progress_interval_ms`] 时，汇报一次（每轮至多
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
    /// **每轮**汇报次数上限（护栏）：`0` ⇒ 一次都不汇报（与 `progress_enabled`
    /// 同效，但语义不同——前者是"这个特性关掉"，后者是"上界为零"）。
    ///
    /// 上界的理由与 `supplements_max_per_drain` 同一条：判定是"该不该说"，
    /// 上界是"最多说几次"。没有它，一个长时间运行的任务会按间隔反复刷屏。
    #[serde(default = "default_progress_max_per_turn")]
    pub progress_max_per_turn: u32,
    /// 汇报的**最少轮次**：本轮至少走完这么多轮工具，才值得说一句。
    ///
    /// 与 [`Self::progress_interval_ms`] / [`Self::progress_max_per_turn`] 同属
    /// 「什么时候该打断」的三个上界（第一个轮边界就够格 ⇒ `1`）。
    #[serde(default = "default_progress_min_rounds")]
    pub progress_min_rounds: u32,
    /// 汇报债行的**派发租约**（毫秒）。这是策略，不是机制常数：`session.progress`
    /// 行把它原样交给 `DebtSpec`，本文件是它的唯一 owner。
    #[serde(default = "default_progress_lease_ms")]
    pub progress_lease_ms: u64,
    /// 汇报债行的**最多尝试次数**（含首次）。默认 1：一句旁白重复说比漏说更糟。
    #[serde(default = "default_progress_max_attempts")]
    pub progress_max_attempts: u32,
    /// 建树策略（`grouped` / `flat`，出厂默认 `grouped` = 今天行为；见 [`PresentationTree`]）。
    ///
    /// 后端组树时决定本轮节点的 `parent_id` 挂哪，前端只按 `parent_id` 插树、
    /// **不知道这个开关存在**——这是该判据的核心：切档前端零代码改动。
    /// 认不出的写法按出厂默认处理（手写 `PLUGIN.yml` 写错不该改变组树），与
    /// `SettingConfig::verbosity` 同一条读侧容错纪律。`deep` 档随子会话转播桥
    /// 改造在 149-ii 接线。
    #[serde(default = "default_presentation_tree")]
    pub presentation_tree: String,
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

pub fn default_presentation_tree() -> String {
    PresentationTree::Grouped.as_str().to_string()
}

pub fn default_max_messages() -> usize {
    100
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
pub fn default_progress_max_per_turn() -> u32 {
    // 5 次：一个 10 分钟的任务最多被打断 5 次，间隔已由 `progress_interval_ms` 保证。
    5
}
pub fn default_progress_min_rounds() -> u32 {
    // 1 轮：避免刚开工就汇报；走完第一轮工具才值得说一句。
    1
}
pub fn default_progress_lease_ms() -> u64 {
    // 60s：与其它行债同一个量级；汇报本身是零往返的填表，派发不会久拖。
    60_000
}
pub fn default_progress_max_attempts() -> u32 {
    // 1：汇报不重试，失败即跳过——重复说一句比漏说更糟。
    1
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
            context_messages: default_context_messages(),
            prune_tool_history: default_prune_tool_history(),
            memory_max_bytes: default_memory_max_bytes(),
            memory_inject_max_bytes: default_memory_inject_max_bytes(),
            supplements_enabled: default_supplements_enabled(),
            supplements_max_per_drain: default_supplements_max_per_drain(),
            opening_enabled: default_opening_enabled(),
            progress_enabled: default_progress_enabled(),
            progress_interval_ms: default_progress_interval_ms(),
            progress_max_per_turn: default_progress_max_per_turn(),
            progress_min_rounds: default_progress_min_rounds(),
            progress_lease_ms: default_progress_lease_ms(),
            progress_max_attempts: default_progress_max_attempts(),
            presentation_tree: default_presentation_tree(),
            approval_grants: ApprovalGrants::default(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
