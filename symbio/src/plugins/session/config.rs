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
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
