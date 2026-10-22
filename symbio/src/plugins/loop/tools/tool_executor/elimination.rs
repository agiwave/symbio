//! 淘汰回路的闸门判据（S13 §3，T-073）：部署级覆盖层批首读一次，`agent_run` 前纯判定。

use std::sync::Arc;

use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt};

/// 淘汰闸门的**判据**（S13 §3，[roadmap/S13 §2](../../../../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")）：
/// 本会话主体是否已被收回 `assign.work`。
///
/// 与 [`super::breaker::BreakerInputs`] 同一条纪律：判据在**批首**读一次，
/// 循环里每个调用点只做一次纯判定。不同的是扫描范围——
///
/// ## 为什么是**部署级**、又为什么是**一个值**
///
/// 裁决（`verdict.asserted`）落在**哪个会话**的 WAL 里取决于违约发生在哪次对话，
/// 而「这个主体不再被授予 `assign.work`」是关于**主体**的事实，不是关于那个会话的：
/// 在 A 会话违约被淘汰的主体，到 B 会话同样不许再派活。按会话扫就会得到
/// 「换个会话就能绕开淘汰」的静默面——闸门形同虚设，而每条断言都还写着它成立。
///
/// 部署级遍历（全会话、含嵌套子会话、读完整史）因此留在**持有全部事实源的
/// session**（动作面 `elimination-revoked` 折叠一次）；驱动器只收结论值
/// （[ADR-086 §4](../../../../../../docs/010-adr/086.md "ADR-086")：全量事件过墙是
/// P6「值注入」的反面）。本结构因此只剩「主体派生 + 结论搬运 + 纯判定」。
pub struct EliminationInputs {
    /// 判据指向的主体（本会话选定的 agent；消息与日志里要点名）。
    pub principal: String,
    /// 该主体是否已被收回 `assign.work`（覆盖层 last-wins，fail-closed 的反面：
    /// 只有一条**生效的撤销**才为真——恢复事件会让它翻假）。
    pub delegation_revoked: bool,
}

impl EliminationInputs {
    /// 批首读一次判据：主体派生 + 搬运 session 折好的撤销结论。
    pub fn of(delegation_revoked: bool, ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let principal =
            crate::symbio_core::principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref());
        Self {
            principal,
            delegation_revoked,
        }
    }
}

#[cfg(test)]
#[path = "elimination.test.rs"]
mod tests;
