//! 跨插件共享的名字 —— **词汇层**。
//!
//! 名字住这里，实现住各自插件；这是插件之间唯一可 import 的共同住处（D-364）。
//! 事件名与跨插件路由名另有清单投影：`events.rs` / `routes.rs` 由各 owner 的
//! `PLUGIN.yml` 生成，消费方只引常量；视图名的公开动作是 owner 自己的报表登记
//! （`submit_reports!`，D-387），没有生成清单。本文件只放
//! **无法从清单投影**的共享名：端口名、跨插件的行行为名、VDFS 路径段、详情表单
//! 条件键，以及治理能力名的门面别名。
//!
//! 判据：同一个**有结构**的名字（含 `.` / `/`）出现在两个非 `plugin-api` 的插件
//! crate 里即红；同名 `const` 在两个插件各声明一次也红（`scripts/audits/shared-name-audit.mjs`
//! 的 SV-001）。新增共享名 = 在这里加一条，两侧 import；只在单个插件内出现的名字
//! 不要搬进来——那是实现自报的词汇（D-363）。
//!
//! `PORT_TOOL` / `SEG_INBOX` 是**多义拼写**：`"tool"` 同时是消息角色、`"inbox"`
//! 同时是消息种类。字面量扫描会把不同概念判成重复，因此它们只由「同名 const 在
//! 两个 crate」那一半守；消费方仍应 import，不要写字面量。

/// 一次性问答端口：`model` 声明，`classify` / `compact` / `compose` / `notes` /
/// `session` 调用。
pub const PORT_MODEL_ASSIST: &str = "model.assist";

/// 工具端口：`tool` 声明，`model` 在债表里等它。
pub const PORT_TOOL: &str = "tool";

/// 模型端口：`model` 声明，本轮往返的 `response` / `failed` 载荷 `port` 取它。
///
/// 与 [`PORT_TOOL`] 同属**多义拼写**（载荷里的 `model` 字段是模型 id），字面量扫描
/// 会把不同概念判成重复，因此只由「同名 const 在两个 crate」那一半守；消费方 import，
/// 不要写字面量。
pub const PORT_MODEL: &str = "model";

/// `todo_write` 的落格行为：`local` 声明工具，`task` 实现行为。
pub const BEHAVIOR_TASK_TABLE: &str = "task_table";

/// 会话收件箱路径段：`agent` / `heartbeat` / `telegram` / `session` 共用。
pub const SEG_INBOX: &str = "inbox";

/// 详情表单的能力位条件键：前端把 `cap.<能力>` 解析成表单能力表
/// （`tauri/src/components/vdfs/DetailForm.vue`）；`mcp` 与 `model` 的详情动作
/// 各写一次同一个键，故它的 Rust 侧住处在这里。
pub const COND_CAP_TEST_CONNECTION: &str = "cap.test_connection";

/// 派活能力名（治理层 `Capability::AssignWork` 的书写形态）。
pub const CAP_ASSIGN_WORK: &str = symbio_sdk::authz::CAP_ASSIGN_WORK;
