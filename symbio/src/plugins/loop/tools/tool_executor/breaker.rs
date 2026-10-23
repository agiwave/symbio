//! 外部执行闸门的判据（S8 第 20 步）：批首读一次授权与成本台账，调用点只做纯判定。

use std::sync::Arc;

use crate::symbio_core::{Event, LatencyTier, PluginInvokeRequest, PluginInvokeRequestExt};

/// 熔断闸门的**判据**（S8 第 20 步，[roadmap/S09 §6](../../../../../../docs/001-goal/129-外部执行与熔断.md "GO-129-外部执行与熔断")）。
///
/// 判据在**批首**读一次，随后每个调用点只做一次纯判定：授权表与成本台账都是
/// 只读的，闸门不该为了拿到自己的判据就把整个事实源重放进工具循环
/// （一次工具批里几十个调用点 × 全量 WAL 重放 = 闸门自己变成时延源）。
///
/// 三个毫秒数的口径（D-044：实测与判据同源，**读台账不读运行时计数器**）：
///
/// | 形参 | 取值 | 为什么是它 |
/// |---|---|---|
/// | `authorized` | 授权矩阵按能力名判 `produce.artifact` | 未持外部执行能力 ⇒ `Refuse`（**零事件**，验收 1） |
/// | `spent_ms` | `cost_ledger` 按主体累计（读**会话目录**的 WAL） | 「已耗多少」的唯一真源，与 `session/stats` 的成本列同一份切片 |
/// | `requested_ms` | 深度档预算 | 工具循环所在档，本轮**申请**这么多 |
/// | `budget_ms` | 自主层预算 | 会话累计成本**总预算**——四层时延的第四个取值复用为天花板 |
///
/// `budget_ms` 是**初值**不是定值：[04 §1](../../../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")
/// 写明「所有百分比与毫秒数是初值，必须在阶段一埋点后校准」。它长什么样由
/// 判据表回答，不由调用点回答——所以三个数都收在本结构里，测试可以**只换预算**
/// 就看见判定翻面（S09 §6.4 反向用例：预算在生效而非常量）。
pub struct BreakerInputs {
    /// 主体是否持外部执行能力（读路径判定）。
    pub authorized: bool,
    /// 已耗（会话累计，`cost_ledger` 台账）。
    pub spent_ms: u64,
    /// 本次申请（工具循环所在档的额度）。
    pub requested_ms: u64,
    /// 总预算（会话累计成本天花板）。
    pub budget_ms: u64,
}

impl BreakerInputs {
    /// 批首读一次判据：授权走矩阵（fail-closed），已耗走台账（D-044）。
    ///
    /// 台账读不到 / 还没有事实源 ⇒ **零账**，不是故障——「还没记过账」不等于
    /// 「不许执行」，闸门因为没有账就把工具全拦下才是静默失效。
    pub fn of(snapshot: &[Event], ctx: &Arc<dyn PluginInvokeRequest>) -> Self {
        let principal =
            crate::symbio_core::principal_of(ctx.get(crate::symbio_core::AGENT_ID).as_deref());
        let authorized = crate::symbio_core::production_matrix()
            .can_write_name(&principal, crate::symbio_core::CAP_EXTERNAL_EXECUTION);
        Self {
            authorized,
            spent_ms: session_cost_ms(snapshot, &principal),
            requested_ms: LatencyTier::Deep.budget_ms(),
            budget_ms: LatencyTier::Autonomic.budget_ms(),
        }
    }
}

/// 会话累计已耗（毫秒）：`cost_ledger` 台账按主体取数。
///
/// 与 `session/stats` 的成本列**同一切片同一投影**——两处各算一遍必然漂移成
/// 「闸门说还有余量、出口说已耗尽」，而错的若是台账，熔断就形同虚设（D-044）。
///
/// ## 从切片取账，不再自己开档
///
/// 切片由会话存储层交回（D-084 §1-A）：读方拿的是事实的值，不是路径。
/// 历史教训：曾按 `<插件目录>/v2-events.wal` 直接 join（漏了会话 id 那层），
/// `open_readonly` 失败即返回 0 ⇒ `spent_ms` 恒 0 ⇒ 闸门的预算分支永远走不到，
/// 而**没有任何东西会变红**。把「读哪份事实」收进存储层后，这类错位在类型上消失。
fn session_cost_ms(snapshot: &[Event], principal: &str) -> u64 {
    // 成本台账是**会话累计**口径（S2.7-b2）：被快照取代而移入归档段的花费也在
    // `range(0)` 里 ⇒ 不会**静默**少算压缩前的成本（闸门说「还有余量」而实际已耗尽，
    // D-044 的熔断随之形同虚设）。
    crate::symbio_core::cost_ledger()
        .read_all(snapshot)
        .value
        .of(principal)
        .spent_ms
}
