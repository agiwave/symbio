//! 会话的纯事实记录与汇报构造。
//!
//! `ReportDraft` 由本插件写侧与重建侧共用，构造出的 ChatNode 经
//! [`super::node_build::dialog_node`] 保持同一形状。任务声明类型归 loop owner，
//! 插件间共享的词表由 SDK 提供。

use super::node_build::dialog_node;
use symbio_platform::kernel::chat::chat_node::ChatNode;

use serde::{Deserialize, Serialize};
use symbio_platform::kernel::event::Event;

/// 一句汇报的草稿：文本 + 它据以生成的运行现状。
///
/// 写侧由批次边界的汇报执行体（`loop::report_exec`）产出，读侧由重建引擎
/// （`rebuild`）从 `turn × asserted` 事件载荷**重建同一格**——两头都经
/// [`Self::to_message`] 这一个构造，复抄形状的代价是已知的：上一版落库侧抄漏
/// `msg_type`，节点落了库却不成形状。两份真源不可能，因为内容由同一个草稿生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportDraft {
    /// 汇报正文（compose 的模板产线，零 LLM 往返）。
    pub text: String,
    /// 说这句话时已完成了几轮工具调用。
    pub tool_rounds: usize,
    /// 说这句话时距上次「用户看得见的一句话」多久。
    pub quiet_ms: i64,
}

// ==================== 跨墙纯值记录（ ====================
//
// 驱动器切成独立插件后，这些记录在 session 与 loop 两侧都要命名。它们只承载
// **值**（存储读出的快照、进展投影、抽干结果），不含任何存储实现或宿主状态。

/// 一次部署级扫描交回的**值**：这份事实源是不是调用方自己的会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSnapshot {
    pub own: bool,
    pub events: Vec<Event>,
}

/// 折进消息的元数据键（写侧、重建侧与读侧共用；session 的 `rebuild` 需要它们）。
pub const META_SUPPLEMENT: &str = "supplement";
pub const META_SUPPLEMENT_IDS: &str = "supplement_ids";
pub const META_SUPPLEMENT_COUNT: &str = "supplement_count";

impl ReportDraft {
    /// 落库镜像上的那一格（前端读它）：`dialog_node(text, "progress", true)`。
    ///
    /// 为什么**不复抄** `dialog_node` 的构造：构造只有一份，漂移才不可能；
    /// `SURFACE_REPLY` / `REASON_PROGRESS` 两个常量留在各自判定点那侧，
    /// 由函数签名（`reason: &str`）承担跨模块契约。
    pub fn to_message(
        &self,
        principal: &symbio_platform::kernel::chat::principal::Principal,
    ) -> ChatNode {
        dialog_node(&self.text, "progress", true, principal)
    }
}
