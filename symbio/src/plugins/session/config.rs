//! 会话插件的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 为什么在插件里，而不是 `symbio_core::schemas`
//!
//! 它曾经住在 `symbio_core::schemas::session::session_config`，与同目录的
//! `session_chat` / `chat_message` 并列。但两者性质不同：
//!
//! - **协议 schema**（那两个）是**跨插件契约**——agent / local / model 都按同一份
//!   定义拼 WS 帧与 payload，改了会同时影响多方，因此归核心；
//! - **本文件是配置**：全仓引用**只在 `src/plugins/session/` 内**，前端与 CLI 零镜像，
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
/// - `store_kind`（及其 `StoreKind` 枚举）：曾经用它在 `file` / `sqlite` / `memory`
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
    /// 最大工具调用迭代轮数（**0 = 不限制**，且 0 即默认值）
    ///
    /// 语义与 `model_chat::Request::max_tool_rounds` 对齐：session 编排层仅在
    /// 本值 > 0 时才下发显式软上限，否则传 `None`（= 无限轮次）。
    /// 用户明确要求不对智能体会话设置硬性轮次上限，故默认取 `0`（显式"不限制"，
    /// 不再用 65535 这类魔法数表达同一意图）；达到软上限时 chat_loop 会先给出
    /// 明确提示再正常退出（非静默熔断）。旧存档中已落盘的 65535 行为等价（实质
    /// 无上限），不受本默认值变更影响。
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// 内容节点淡化行数阈值（请求视图中超过此行数或 token 超预算时做头尾淡化；存储恒为完整原文）
    #[serde(default = "default_compress_line_threshold")]
    pub compress_line_threshold: usize,
    /// 内容节点淡化的"最近 N 条内容节点"保护数（B1 保护窗口）：最近的 Text/Reasoning
    /// 节点在请求视图中豁免淡化（末条消息恒受保护），0 表示不保护
    #[serde(default = "default_compress_keep_recent")]
    pub compress_keep_recent: usize,
    /// 保留完整结果的最近工具调用数量限制（滑动窗口）
    #[serde(default = "default_tool_context_window")]
    pub tool_context_window: usize,
    /// 老旧工具结果淡化（fade）的激活阈值：单轮请求的工具迭代轮数**超过**此值后，
    /// 请求视图才把较早轮次的工具结果压成头尾摘要。
    ///
    /// 与 `chat_loop` 里的旧常量 `FADE_ACTIVATE_ROUNDS` 同源（原为硬编码 40）；
    /// 统一进本结构后，fade 的全部参数都只有这一个真源。置 0 等价于"每一轮都启用
    /// fade"（判定是 `tool_rounds > 阈值`），要彻底关闭请保持默认值或调大。
    #[serde(default = "default_fade_activate_rounds")]
    pub fade_activate_rounds: usize,
    /// fade 的保留窗口：最近 N 个 user turn 的工具结果保持原文，更早的才淡化
    ///（原硬编码常量 `FADE_KEEP_RECENT_TURNS` = 12）。
    #[serde(default = "default_fade_keep_recent_turns")]
    pub fade_keep_recent_turns: usize,
    /// 是否启用工具压缩：向模型暴露主动压缩工具（context_compact）并允许水位
    /// 提醒引导模型调用（独立于自动压缩开关）
    #[serde(default = "default_enable_compact_tool")]
    pub enable_compact_tool: bool,
    /// 写入期工具链裁剪：是否在**落库时**物理删除 `context_messages` 轮之前的
    /// Tool / ToolCall / Reasoning 节点及其存档文件。
    ///
    /// 这是「存储保持完整原文、压缩只发生在请求视图」架构原则的**唯一例外**，
    /// 存在理由仅是控制工具密集型会话的磁盘与节点树体积（token 治理已由
    /// `build_request_view` 的骨架化/淡化承担，本开关不影响发给模型的上下文大小）。
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
    /// ## 为什么出厂默认为 `true`
    ///
    /// 它是**用户可见层面**的一部分（与 `classify_enabled` / `compose_enabled` 同一条
    /// 判据）：一个跑了几分钟的任务，界面上什么都不说，用户无法区分"在干活"与
    /// "卡死了"。而它的代价被三个上界钉住（间隔 / 最少轮次 / 每轮次数），
    /// 不会变成噪声源。
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
pub fn default_max_tool_rounds() -> usize {
    0 // 0 = 不限制（产品决策：不对智能体会话设硬性轮次上限）
}
pub fn default_compress_line_threshold() -> usize {
    200
}
pub fn default_compress_keep_recent() -> usize {
    3
}
pub fn default_tool_context_window() -> usize {
    15
}
pub fn default_fade_activate_rounds() -> usize {
    40
}
pub fn default_fade_keep_recent_turns() -> usize {
    12
}
pub fn default_enable_compact_tool() -> bool {
    false
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
    // 用户可见层面的一部分：一个跑了几分钟的任务不该在界面上什么都不说。
    true
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

impl SessionConfig {
    /// 下发给 `model_chat::Request::max_tool_rounds` 的值（契约翻译点）。
    ///
    /// `SessionConfig::max_tool_rounds == 0` 表示**不限制** → 传 `None`（chat_loop 中
    /// `None` = 无限轮次）；> 0 才是显式软上限。此前编排层三处无条件 `Some(...)`，
    /// 使 `None` 语义在主路径不可达，且 0 会变成"0 轮即熔断"的错误行为。
    pub fn model_chat_max_tool_rounds(&self) -> Option<usize> {
        (self.max_tool_rounds > 0).then_some(self.max_tool_rounds)
    }

    /// 生效的会话记忆**写入**上限（下界 1 字节，避免配成 0 后一切写入都失败却看不出原因）
    pub fn effective_memory_max_bytes(&self) -> usize {
        self.memory_max_bytes.max(1)
    }

    /// 生效的会话记忆**注入**预算（下界 1 字节，理由同上）
    pub fn effective_memory_inject_bytes(&self) -> usize {
        self.memory_inject_max_bytes.max(1)
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_messages: default_max_messages(),
            auto_compress: default_auto_compress(),
            context_messages: default_context_messages(),
            max_tool_rounds: default_max_tool_rounds(),
            compress_line_threshold: default_compress_line_threshold(),
            compress_keep_recent: default_compress_keep_recent(),
            tool_context_window: default_tool_context_window(),
            fade_activate_rounds: default_fade_activate_rounds(),
            fade_keep_recent_turns: default_fade_keep_recent_turns(),
            enable_compact_tool: default_enable_compact_tool(),
            prune_tool_history: default_prune_tool_history(),
            memory_max_bytes: default_memory_max_bytes(),
            memory_inject_max_bytes: default_memory_inject_max_bytes(),
            supplements_enabled: default_supplements_enabled(),
            supplements_max_per_drain: default_supplements_max_per_drain(),
            classify_enabled: default_classify_enabled(),
            compose_enabled: default_compose_enabled(),
            progress_enabled: default_progress_enabled(),
            progress_interval_ms: default_progress_interval_ms(),
            progress_min_rounds: default_progress_min_rounds(),
            progress_max_per_turn: default_progress_max_per_turn(),
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
