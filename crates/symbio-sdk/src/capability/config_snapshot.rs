//! 当轮配置快照的贡献面（跨插件共享设施）。
//!
//! ## 这一面回答什么
//!
//! 「这一格开关是谁的」与「决策什么时候读它」是两件事，此前被绑成了一件：
//! 宿主把整份配置定义平铺投影成一条事实，于是每加一格能力开关都要改宿主的
//! 配置结构、表单与投影。本面把两者切开——**真源在能力包，读法仍是事实**：
//!
//! - 能力插件从自己的配置文档读值，在既有的收集广播里声明「本轮快照要贡献哪一格」；
//! - 宿主在开轮时只按 owner 收集并合并，不认识任何键名、不认识任何能力；
//! - 决策面仍只读**开轮落下的那条事实**，读不到就用出厂值。生效时刻单一，
//!   与「决策只读快照」的既有纪律一致。
//!
//! ## 为什么复用能力表，而不是新开一条通道
//!
//! 族是值而不是内核枚举：常量与载荷类型住本域，内核零改动。收集器沿用
//! [`CAPABILITY_VISITOR`](super::CAPABILITY_VISITOR) 与开轮时那一次 `traverse`——
//! 不新增上下文键、不新增实体、不新增事件类型。插件侧只有一个入口，
//! 与 `capability_announce_configurable` 同形；宿主侧只有一个读口。
//!
//! ## 边界
//!
//! - 本面只交**值**，不交配置类型：能力包之间、能力包与宿主之间没有类型依赖。
//! - 同名后注册覆盖先注册（表语义），与主能力表的 `put` 一致；不按名字做特殊处理。
//! - 卸载插件 ⇒ 该格缺席 ⇒ 读方按自己的出厂值收敛，不留静默边。

use std::sync::Arc;

use serde_json::{Map, Value};
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

use super::CAPABILITY_VISITOR;

/// 配置快照族。**族是值**：新族不需要内核枚举，也不需要改动任何语法面。
pub const CONFIG_SNAPSHOT: CapabilityFamily = CapabilityFamily::multi("config.snapshot");

/// 一格贡献的载荷。私有 newtype：插件只经 [`capability_announce_config_snapshot`]
/// 交值，不接触载荷类型，也就不会与宿主共享一个配置结构。
struct ConfigSnapshotValue(Value);

/// 插件侧唯一入口：声明本轮要贡献的一格。
///
/// 收集器缺席（不在能力广播里）时静默跳过——贡献是增益，不影响插件自身行为。
/// `key` 由**声明方**给，宿主不解释；值来自声明方自己的配置文档。
pub async fn capability_announce_config_snapshot(
    ctx: &Arc<dyn PluginInvokeRequest>,
    key: impl Into<String>,
    value: Value,
) {
    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
        visitor
            .put(CapabilityRow::new(
                CONFIG_SNAPSHOT,
                key,
                ConfigSnapshotValue(value),
                true,
            ))
            .await;
    }
}

/// 宿主侧唯一读口：按注册序把贡献折成一张 JSON 对象。
///
/// 宿主只做合并，不认识键名、不校验值、不补缺省——缺省是**读方**的事。
/// 取不到收集器或没有贡献时返回空表，开轮事实照常落格。
pub async fn capability_config_snapshot(visitor: &dyn Registry) -> Map<String, Value> {
    let mut snapshot = Map::new();
    for row in visitor.list(CONFIG_SNAPSHOT).await {
        if let Some(value) = row.payload::<ConfigSnapshotValue>() {
            snapshot.insert(row.name().to_string(), value.0.clone());
        }
    }
    snapshot
}

#[cfg(test)]
#[path = "config_snapshot.test.rs"]
mod tests;
