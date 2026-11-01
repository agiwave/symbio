//! 外部执行闸门的判据：批首读一次授权与成本台账，调用点只做纯判定。
//!
//! ## 为什么它留在这里（[DE-039 D-271](../../../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断")）
//!
//! 名册第 8「副作用分级」判过「拆」，裁决是**不迁**：判据的两个输入一个在 E 类
//! （`production_matrix()` 的封闭语法，能力包读不到）、一个是一条**声明视图**（名册 #18
//! 的 `cost.ledger`，折叠住成本能力包，本模块按名字读），而熔断要的两个出口——拒绝且
//! **零事件**、熔断**必须**入格 `control × opened`——在 I-3 的 `BeforeRefusal`
//! 形状之外（它只说得出「可审计拒绝」）。迁它等于新开一格，不是迁移。

use std::sync::Arc;

use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::event::Event;
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

/// 熔断闸门的**判据**（[DE-039 §6](../../../../../docs/003-design/039-外部执行与熔断.md "DE-039-外部执行与熔断")）。
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
/// | `spent_ms` | 声明视图 `cost.ledger` 按主体累计（读**会话目录**的 WAL） | 「已耗多少」的唯一真源，与 `session/stats` 的成本列同一份切片 |
/// | `requested_ms` | 深度档预算 | 工具循环所在档，本轮**申请**这么多 |
/// | `budget_ms` | 自主层预算 | 会话累计成本**总预算**——四层时延的第四个取值复用为天花板 |
///
/// `budget_ms` 是**初值**不是定值：[DE-016 §8](../../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")
/// 写明「所有百分比与毫秒数都是初值」。它长什么样由
/// 判据表回答，不由调用点回答——所以三个数都收在本结构里，测试可以**只换预算**
/// 就看见判定翻面（DE-039 §6.4 反向用例：预算在生效而非常量）。
pub struct BreakerInputs {
    /// 主体是否持外部执行能力（读路径判定）。
    pub authorized: bool,
    /// 已耗（会话累计，`cost.ledger` 台账）。
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
            symbio_sdk::authz::principal_of(ctx.get(symbio_sdk::keys::AGENT_ID).as_deref());
        let authorized = symbio_sdk::authz::production_matrix()
            .can_write_name(&principal, symbio_sdk::authz::CAP_EXTERNAL_EXECUTION);
        Self {
            authorized,
            spent_ms: session_cost_ms(snapshot, &principal),
            requested_ms: LatencyTier::Deep.budget_ms(),
            budget_ms: LatencyTier::Autonomic.budget_ms(),
        }
    }
}

/// 成本台账的**声明视图名**（owner = 成本能力包，名册 #18）。
///
/// 闸门按名字读，不抄口径：折叠住在 owner 那一侧，与出口 `stats.cost` 读的是**同一条算式**。
pub(crate) const COST_LEDGER_VIEW: &str = "cost.ledger";

/// 日志通道：本闸门跑在会话子树里，读数落在会话那条通道（与 `memory_events` 同一口径）。
const LOG_CHANNEL: &str = symbio_deploy::PLUGIN_ID_SESSION;

/// 会话累计已耗（毫秒）：读 `cost.ledger` 里这个主体的 `spent_ms`。
///
/// ## 与出口同一条算式
///
/// `session/stats` 的成本列读的是**同一个视图名**——两处各折一遍必然漂移成「闸门说还有
/// 余量、出口说已耗尽」，而错的若是台账，熔断就形同虚设（D-044）。折叠因此只有一个
/// owner（名册 #18 搬进能力包），两个读者。
///
/// ## 从切片取账，不再自己开档
///
/// 切片由会话存储层交回（D-084 §1-A）：读方拿的是事实的值，不是路径。
/// 历史教训：曾按 `<插件目录>/v2-events.wal` 直接 join（漏了会话 id 那层），
/// `open_readonly` 失败即返回 0 ⇒ `spent_ms` 恒 0 ⇒ 闸门的预算分支永远走不到，
/// 而**没有任何东西会变红**。把「读哪份事实」收进存储层后，这类错位在类型上消失；
/// 今天的「读不到」只剩读口的四种缺席态，每一条都落一行 warn——零账不再是静默的。
fn session_cost_ms(snapshot: &[Event], principal: &str) -> u64 {
    // 成本台账是**会话累计**口径：被快照取代而移入归档段的花费也在
    // `range(0)` 里 ⇒ 不会**静默**少算压缩前的成本（闸门说「还有余量」而实际已耗尽，
    // D-044 的熔断随之形同虚设）。
    // `value` 必须是**函数作用域的局部**：下面按主体取的那一行是它借出来的切片。
    let value = match symbio_plugin_api::read_shared_view_all(COST_LEDGER_VIEW, snapshot) {
        Ok(value) => value,
        Err(why) => {
            // 读不到 ⇒ 零账，但**必须出声**：静默的 0 就是上面那条历史事故换了个来源。
            symbio_plugin_api::plugin_warn!(
                LOG_CHANNEL,
                "[Breaker] 成本台账 `{COST_LEDGER_VIEW}` 读不到（{why}）：本轮按零账判定"
            );
            return 0;
        }
    };
    let Some(rows) = value
        .get("by_principal")
        .and_then(serde_json::Value::as_object)
    else {
        // 形状读不出 = 版本对不上，那是装配问题：出声，不假装「没有账」。
        symbio_plugin_api::plugin_warn!(
            LOG_CHANNEL,
            "[Breaker] 成本台账 `{COST_LEDGER_VIEW}` 的读数没有 `by_principal` 这张表：本轮按零账判定"
        );
        return 0;
    };
    // 书里没有这一行 = 这个主体还没记过账，**正常路径**（空会话、别的主体单独花的钱）。
    rows.get(principal)
        .and_then(|row| row.get("spent_ms"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}
