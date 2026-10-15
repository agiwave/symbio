//! 淘汰回路的闸门判据（S13 §3，T-073）：部署级覆盖层批首读一次，`agent_run` 前纯判定。

use std::sync::Arc;

use crate::symbio_core::{EventWalStore, PluginInvokeRequest, PluginInvokeRequestExt};

/// 淘汰闸门的**判据**（S13 §3，[roadmap/S13 §2](../../../../../../docs/003-stage/013-多智能体社会.md "ST-013-多智能体社会")）：
/// 本会话主体是否已被收回 `assign.work`。
///
/// 与 [`super::breaker::BreakerInputs`] 同一条纪律：判据在**批首**读一次，
/// 循环里每个调用点只做一次纯判定。不同的是扫描范围——
///
/// ## 为什么是**部署级**（扫全部会话的 WAL）
///
/// 裁决（`verdict.asserted`）落在**哪个会话**的 WAL 里取决于违约发生在哪次对话，
/// 而「这个主体不再被授予 `assign.work`」是关于**主体**的事实，不是关于那个会话的：
/// 在 A 会话违约被淘汰的主体，到 B 会话同样不许再派活。按会话扫就会得到
/// 「换个会话就能绕开淘汰」的静默面——闸门形同虚设，而每条断言都还写着它成立。
///
/// 遍历形状与 `stats` 的实例聚合同一条先例（`fleet_reading`）：存储根（本插件
/// 自己的目录）的**直接子目录**各是一份会话 WAL，只走一层；打不开的跳过并出声；
/// 读 `history()` 而非 live 段——裁决若被压缩归档进历史段，只读 live 会把它**静默
/// 漏掉**，那等于替一个已被淘汰的主体恢复了派活权。
pub struct EliminationInputs {
    /// 判据指向的主体（本会话选定的 agent；消息与日志里要点名）。
    pub principal: String,
    /// 该主体是否已被收回 `assign.work`（覆盖层 last-wins，fail-closed 的反面：
    /// 只有一条**生效的撤销**才为真——恢复事件会让它翻假）。
    pub delegation_revoked: bool,
}

impl EliminationInputs {
    /// 批首读一次判据：主体派生 + 部署级覆盖层折叠 + 撤销判定。
    pub fn of(session_dir: &std::path::Path, ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let principal = crate::symbio_core::authz::principal_of(
            ctx.get(crate::symbio_core::AGENT_ID).as_deref(),
        );
        let delegation_revoked = revoked_overlay(session_dir)
            .is_revoked(&principal, crate::symbio_core::authz::CAP_ASSIGN_WORK);
        Self {
            principal,
            delegation_revoked,
        }
    }
}

/// 部署级覆盖层：扫存储根下全部会话 WAL，把历次 `verdict.asserted` 折成
/// 「现在谁被收回了什么」。
///
/// 载荷 ↔ 变更的映射不在本函数复述：它在 core 的
/// [`crate::symbio_core::EliminationDecider::verdict_grants_changes`]（单点定义）——
/// 本函数只做遍历 + 开档 + 交出事件流水，与 `fleet_reading` 同形。
fn revoked_overlay(session_dir: &std::path::Path) -> crate::symbio_core::GrantsOverlay {
    let mut events = Vec::new();
    if let Ok(entries) = std::fs::read_dir(session_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let wal = entry.path().join(super::super::super::paths::V2_WAL_FILE);
            if !wal.exists() {
                continue;
            }
            let Ok(store) = EventWalStore::open_readonly(&wal) else {
                crate::plugin_warn!(
                    "session",
                    "[Tool] 淘汰闸跳过打不开的 WAL：{}",
                    wal.display()
                );
                continue;
            };
            // `history()`：累计口径（见结构文档——归档段的裁决不许被静默漏掉）。
            events.extend(store.history());
        }
    }
    crate::symbio_core::GrantsOverlay::fold(
        crate::symbio_core::EliminationDecider::verdict_grants_changes(&events),
    )
}

#[cfg(test)]
#[path = "elimination.test.rs"]
mod tests;
