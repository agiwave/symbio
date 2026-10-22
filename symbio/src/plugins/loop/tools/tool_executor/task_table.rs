//! 任务表写侧闸（S03 §3，T-107）：`todo_write` 前的授权判据——**立任务
//! （`define.work`）或独立判定（`assert.verification`）二居其一**。
//!
//! 与 [`super::elimination`] 同一条纪律：判据在**批首**读一次（主体派生 + 矩阵谓词），
//! 循环里每个调用点只做一次纯判定。不同的是扫描范围——本闸只问**本会话主体**那一行
//! （任务表是会话自己的工作跟踪，写它的资格随主体走，不随部署状态变）。

use std::sync::Arc;

use crate::symbio_core::PluginInvokeRequest;
use crate::symbio_core::{
    principal_of, production_matrix, PluginInvokeRequestExt, CAP_ASSERT_VERIFICATION,
    CAP_DEFINE_WORK,
};

/// 任务表写侧闸的**判据**（[roadmap/S03 §3](../../../../../../docs/009-task/123-多步任务与返工.md "T-123-多步任务与返工")）：
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
    /// [`crate::symbio_core::CAP_DEFINE_WORK`] / [`crate::symbio_core::CAP_ASSERT_VERIFICATION`] 两处常量
    /// 同一套拼写（表授予 A、闸门判 B 的漂移面由此收窄到一处）。
    pub fn of(ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let principal = principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref());
        let matrix = production_matrix();
        let may_write = matrix.can_write_name(&principal, CAP_DEFINE_WORK)
            || matrix.can_write_name(&principal, CAP_ASSERT_VERIFICATION);
        Self {
            principal,
            may_write,
        }
    }
}

// 「谁是任务表写入」这个判据不在本文件：它在工具自己的
// `CapabilityMeta::owns_task_table` 上自声明（T-158），执行器经
// [`ToolDecls`](super::ToolDecls) 读声明——把名字抄进闸门的那份副本
// （`TODO_WRITE_TOOL` / `is_task_table_tool`）随机制化一并退役。

#[cfg(test)]
#[path = "task_table.test.rs"]
mod tests;
