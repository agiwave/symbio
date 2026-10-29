//! 跨切片**事实记录词汇**：写侧在驱动器（`loop` 片）、落侧/读侧在留守存储层的
//! 那批纯数据记录类型。
//!
//! ## 为什么住 core
//!
//! [D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 4/5 组与 2 组的
//! `ReportDraft` 半句：驱动器切成独立插件（[D-082](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）后，
//! 这些类型隔着 E-009 的插件墙两头都要用——
//!
//! | 类型 | 写侧（驱动器） | 落/读侧（留守） |
//! |---|---|---|
//! | [`ReportDraft`] | `report_exec`（批次边界汇报执行体的产物） | `rebuild::reported_node` 回放汇报格 |
//!
//! 任务表的声明类型**不在这里**：写侧与落侧现在同属 `symbio-plugin-loop`
//! （效果端口把「哪一次调用算声明」交回 owner，DE-025 D-207），类型随 owner 住
//! 那个插件——core 的数据面只装真的跨插件的东西。
//!
//! 与 `types::Session` 一族同判：**存储的记录类型，归属随数据 owner，跨插件者上移**。
//! 本模块只装**数据与纯构造**（[`ReportDraft::to_message`] 经 [`crate::node_build`]
//! 的同一构造器）；记账判据（`note_*` 的门与 `behaviors` 里的 `delegation_promise` 声明）与
//! 派发机制（行表 / 绑定表 / 执行体）都不在这里——它们是机制不是词汇，随各自的 owner 住插件侧。

use crate::chat_node::ChatNode;
use crate::event::Event;
use crate::node_build::dialog_node;
use serde::{Deserialize, Serialize};

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

// ==================== 跨墙纯值记录（D-086 §7） ====================
//
// 驱动器切成独立插件后，这些记录在 session 与 loop 两侧都要命名。它们只承载
// **值**（存储读出的快照、进展投影、抽干结果），不含任何存储实现或宿主状态。

/// 一次部署级扫描交回的**值**：这份事实源是不是调用方自己的会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSnapshot {
    pub own: bool,
    pub events: Vec<Event>,
}

/// Q3 · 单个 worker 的进展投影。
///
/// 三个字段与 B2（前端把「过程段」收起成的那行状态）**逐字同名**：`in_flight` /
/// `rounds` / `steps`。同名约束的是**字段名与语义**，不是算法——后端只见磁盘，
/// 前端还看得见父工具状态（B2 判据 1 的 OR 式），两端各自用可见信息判定同一语义。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkerProgress {
    /// 子会话 id（`metadata.parent_session_id` 归属本会话）。
    pub id: String,
    /// 标题；存量文件缺投影字段时为空，装配时以 [`Self::id`] 兜底。
    pub title: String,
    /// 该 worker 是否仍在运行（末条消息**未见终态**即为真）。
    pub in_flight: bool,
    /// Turn 数 = 其 user 消息数 = **前端过程段 Turn 条数**——B2 判据 2「数值同源」
    /// 的落点。**不能拿 `message_count` 顶**：那是总消息数，与 Turn 数不同量。
    pub rounds: usize,
    /// 非空内容节点产出的步骤；**无内容节点不产出步骤**（B2 判据 3）。
    pub steps: Vec<String>,
}

/// 抽干结果：**正文 + 原条数 + 溯源身份**。
///
/// `InboxItem` 是 session 转写域的私有类型，抽干口不返回它——跨模块暴露内部类型
/// 会让「谁都能构造一个假的抽干结果」成为可能。
///
/// ⚠️ **不带 `ChatNode`**：注入只要正文，落库镜像那条节点由接收方按这里的
/// `id` / `ids` 构造——多带一个字段就是给下一个人留一个不必有的理由。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrainedSupplement {
    /// 折进 prompt 的正文（多条已合并）。
    pub text: String,
    /// **原条数**：合并了 n 条时是 n。
    pub count: u64,
    /// 合并消息的 **id** = **第一条补充的 id**（`merge_supplements` 的既有约定）。
    ///
    /// 为什么必须沿用而不是新造：前端按 id 合并权威帧，换 id 等于让同一句话在前端
    /// **出现两条**。
    pub id: String,
    /// 批内**原始条目 id 列表**（顺序 = 入队顺序），落进 `meta.supplement_ids`。
    pub ids: Vec<String>,
}

/// 折进消息的元数据键（写侧、重建侧与读侧共用；`projection::rebuild` 需要它们）。
pub const META_SUPPLEMENT: &str = "supplement";
pub const META_SUPPLEMENT_IDS: &str = "supplement_ids";
pub const META_SUPPLEMENT_COUNT: &str = "supplement_count";

impl ReportDraft {
    /// 落库镜像上的那一格（前端读它）：`dialog_node(text, "progress", true)`。
    ///
    /// 为什么**不复抄** `dialog_node` 的构造：构造只有一份，漂移才不可能；
    /// `SURFACE_REPLY` / `REASON_PROGRESS` 两个常量留在各自判定点那侧，
    /// 由函数签名（`reason: &str`）承担跨模块契约。
    pub fn to_message(&self, principal: &crate::principal::Principal) -> ChatNode {
        dialog_node(&self.text, "progress", true, principal)
    }
}
