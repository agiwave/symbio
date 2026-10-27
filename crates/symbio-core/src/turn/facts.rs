//! 跨切片**事实记录词汇**：写侧在驱动器（`loop` 片）、落侧/读侧在留守存储层的
//! 那批纯数据记录类型。
//!
//! ## 为什么住 core
//!
//! [D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三 4/5 组与 2 组的
//! `ReportDraft` 半句：驱动器切成独立插件（[plan/14 步 6](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
//! 切片批）后，这些类型隔着 E-009 的插件墙两头都要用——
//!
//! | 类型 | 写侧（驱动器） | 落/读侧（留守） |
//! |---|---|---|
//! | [`Delegation`] | `tools/tool_executor`（`note_delegation` 的出参元素） | `derived_facts::record_derived` 落 `commitment.*` |
//! | [`TaskDeclaration`] / [`TaskItem`] / [`TaskStatus`] | 同上（`note_tasks` 的出参元素） | `task_events` 投影 `task.*` |
//! | [`ReportDraft`] | `report_exec`（批次边界汇报执行体的产物） | `rebuild::reported_node` 回放汇报格 |
//!
//! 与 `types::Session` 一族同判：**存储的记录类型，归属随数据 owner，跨插件者上移**。
//! 本模块只装**数据与纯构造**（[`ReportDraft::to_message`] 经 [`crate::node_build`]
//! 的同一构造器）；记账判据（`note_*` 的门与 `behaviors` 里的 `delegation_promise` 声明）与
//! 派发机制（行表 / 绑定表 / 执行体）都不在这里——它们是机制不是词汇，随各自的 owner 住插件侧。

use crate::chat_node::ChatNode;
use crate::event::Event;
use crate::node_build::dialog_node;
use serde::{Deserialize, Serialize};

/// 本轮的**代际立约记录**（[04 §3.1 批⑧](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")，S08 §3）：
/// 工具执行层把「我向上游承诺了什么、履行了没有」交出来，由轮末收束的
/// `derived_facts::record_derived` 落成 `commitment.*` 格。
///
/// 只认**声明了 `delegation_promise` 行为的工具**（`agent_run` 即是； 改了
/// 判据的住址，没改这条语义）——它是**跨主体**的那一次调用：本会话向上游承诺
/// 「会有另一个智能体接手这件事」。别的工具是本会话自办，办没办成是执行结果，
/// 不是承诺。
///
/// 工具执行层**没有事实源**（它只写节点、不碰 WAL），所以这里是纯出参：
/// 谁负责把承诺入格，谁负责开那扇门（轮末收束的 `derived_facts::record_derived`）。
#[derive(Debug, Clone)]
pub struct Delegation {
    /// 承诺号 = 本次调用在**转写里的 ToolCall 节点 id**（`c-offer-<id>` 由此成格）：
    /// 节点 id 由转写生成、跨轮不撞——模型给的 `tool_call` 编号只在一次响应内唯一，
    /// 直接拿来当事件号会在下一轮撞幂等键（`record_derived` 落格时还会给事件号
    /// 加 `v2t-{anchor_id}` 前缀、一轮一个，两层各自挡一种撞法）。
    pub id: String,
    /// 承诺内容 = 委托出去的那句话（`agent_run` 的 `prompt` 参数）。
    pub promise: String,
    /// 是否履行（`true` ⇒ `commitment.released`，`false` ⇒ `.broken` + `why`）。
    pub ok: bool,
    /// 违约原因（`ok = true` 时为空——违约必须带 why，守约不必）。
    pub why: String,
}

/// 任务表声明（S7 步 16–18，[04 §3.1 批⑨](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// 一次**成功**的 `todo_write` 调用所声明的清单状态。
///
/// 与 [`Delegation`] 同一个观察点（工具执行的唯一入口）。任务表的写方不在工具里：
/// `local/todo_write` 是插件，够不到会话的 v2 事实源，而分发点是模型工具调用的
/// **唯一入口**。这里看得见什么，事实源才记得了什么；记的必须是**观察到的声明**，
/// 不是复述。
///
/// 只记声明、不记清单的当前全量：`local/todo_write` 那份清单是纯内存的
/// （见其模块头），事实源要的是同一份状态的持久化——若在此再算一遍全量，
/// 两处各算一次就是两套真相。因此**声明里没提到的任务一律不动**
/// （`merge = true` 的增量语义，以及 `merge = false` 时被整体替换移出清单的项：
/// 后者在事实源里保持未终态——append-only 不替模型编一条「已验收」的假事实）。
pub struct TaskDeclaration {
    /// 本批声明的任务项，顺序与模型给的一致。
    pub items: Vec<TaskItem>,
}

/// 一项任务的声明态（一次 `todo_write` 调用里的一项）。
#[derive(Debug, Clone)]
pub struct TaskItem {
    /// 任务 id。**缺席时按 `task_{i+1}` 兜底**，与 `local/todo_write` 的默认同一条
    /// 格式——两处各写一次是跨插件契约（E-009 禁止 session 直引 local，所以没有
    /// 共享函数可用）：改名必须两处一起改，否则事实源的 id 会与工具清单对不上，
    /// 调度段就会念出模型不认识的 id。
    pub id: String,
    /// 任务内容（入 `task.opened` 的 `goal`）。
    pub goal: String,
    /// 依赖的任务 id。任务图是**数据不是机制**（`-多步任务与返工` §2），
    /// 声明在清单里，成环 / 悬空由读侧 `acyclic_deps` 报。
    pub depends_on: Vec<String>,
    /// 声明状态。
    pub status: TaskStatus,
}

/// 任务状态（与 `local/todo_write` 的 `status` 枚举同集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStatus {
    /// 未开始。
    #[default]
    Pending,
    /// 进行中。
    InProgress,
    /// 已完成（`task.asserted` 的触发态）。
    Completed,
}

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
