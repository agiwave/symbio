//! 任务表写侧闸（S03 §3，T-107）：`todo_write` 前的授权判据——**立任务
//! （`define.work`）或独立判定（`assert.verification`）二居其一**。
//!
//! 与 [`super::elimination`] 同一条纪律：判据在**批首**读一次（主体派生 + 矩阵谓词），
//! 循环里每个调用点只做一次纯判定。不同的是扫描范围——本闸只问**本会话主体**那一行
//! （任务表是会话自己的工作跟踪，写它的资格随主体走，不随部署状态变）。

use std::sync::Arc;

use crate::symbio_core::PluginInvokeRequest;
use crate::symbio_core::{authz, PluginInvokeRequestExt};

/// 任务表写侧闸的**判据**（[roadmap/S03 §3](../../../../../../docs/003-stage/003-多步任务与返工.md "ST-003-多步任务与返工")）：
/// 本会话主体能否写任务表（`todo_write`）。
///
/// ## 为什么是**两项能力 或**
///
/// 任务表上只有两种权威动作，各对应一行角色：
///
/// - **立任务**（`define.work`）——开格 / 推进任务图。持有者：主智能体（自己的工作
///   跟踪）、自主发起者（「欲」升格为任务，`AutonomousInitiator` 先例）、规划者；
/// - **独立判定**（`assert.verification`）——对完成 / 返工盖章。持有者：验证者、仲裁者
///   （T-073 的淘汰回路）。
///
/// 两项**互不可写**（C15：`PermissionMatrix::new` 拒绝同持）⇒ 没有任何一个主体能
/// 同时「自己立、自己验」——「产出者不得自我认证」在任务表这一层是结构性的。
///
/// ## 被拒的是谁
///
/// 纯产出主体（工人 / 对等体——只持 `produce.artifact`）与纯派活主体（市场——只持
/// `assign.work`）：它们**不写任务表**。工人的交付物是产物格，任务的立与验归工作
/// 权威；今天没有任何生产路径让这些主体调 `todo_write`，故此闸今天不改变任何现有
/// 行为——它改变的是**未来了**：角色行落地后（T-107），一个绑 `worker` 的子会话
/// 调 `todo_write` 会被诚实地拒止，而不是静默地以主智能体的资格写下去。
pub struct TaskTableInputs {
    /// 判据指向的主体（本会话选定的 agent；消息与日志里要点名）。
    pub principal: String,
    /// 该主体能否写任务表（持 `define.work` 或 `assert.verification`）。
    pub may_write: bool,
}

impl TaskTableInputs {
    /// 批首读一次判据：主体派生 + 授权表谓词。
    ///
    /// 谓词按**能力名**问（`can_write_name`）：能力闭集与拼写由构造期兜住
    /// （`from_names` 认不出即拒绝整体构造），读侧照单全收——与
    /// [`authz::CAP_DEFINE_WORK`] / [`authz::CAP_ASSERT_VERIFICATION`] 两处常量
    /// 同一套拼写（表授予 A、闸门判 B 的漂移面由此收窄到一处）。
    pub fn of(ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let principal = authz::principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref());
        let matrix = authz::matrix_for(&principal);
        let may_write = matrix.can_write_name(&principal, authz::CAP_DEFINE_WORK)
            || matrix.can_write_name(&principal, authz::CAP_ASSERT_VERIFICATION);
        Self {
            principal,
            may_write,
        }
    }
}

/// 任务表工具名（LLM 可见短名）——`note_tasks` 认的是同一个名字
/// （`local/todo_write` 的线上全名取末段），两处各判一遍迟早漂移成
/// 「出参见短名、闸门认全名」。
pub const TODO_WRITE_TOOL: &str = "todo_write";

/// 该调用是否是任务表写入（`todo_write`，含 `local/` 等线上前缀）。
pub fn is_task_table_tool(name: &str) -> bool {
    name.trim().rsplit('/').next().unwrap_or_default() == TODO_WRITE_TOOL
}

#[cfg(test)]
#[path = "task_table.test.rs"]
mod tests;
