//! 语义记忆的**声明面**：`memory.recalled` 的名字与格位（DE-026 D-264 第 4 条）。
//!
//! ## 名字与格位为什么在这里
//!
//! 一条事实的「名字 + 格位」是它的**词汇**，不是写它那一行代码的附属物。本包是记忆
//! 词汇的 owner（`memory.*` 三格：`encoded` / `consolidated` / `forgotten` 的格位在
//! 控制面，因为它们的写方在控制面且另有读方；`recalled` 两样都归本包——写方是
//! [`super::contributor::SemanticRecall`]，读方只有将来的统计面）。
//!
//! ## 与 `shared_events` 的分界
//!
//! 名字**没有**登记进 `PLUGIN.yml` 的 `shared_events`：那份清单是给「别的插件按名字
//! 取用」的（生成 `symbio_plugin_api::events`），而 `memory.recalled` 今天没有跨插件
//! 读方。等真有第二个读方时，动作是把它登记进清单、改从生成物取——而不是让读方
//! 抄一份字面量（DE-024 D-204）。

use symbio_plugin_api::{Entity, EventDecl, Verb};

/// 检索事实的名字。幂等键的**词干**由宿主算（`recalled-{user_seq}`，D-252）。
pub(crate) const EVENT_MEMORY_RECALLED: &str = "memory.recalled";

/// 本包交出的全部事件格位。
///
/// 一格：`memory × asserted` ——「我召回了这些」是一次断言，不是一次开启或收束。
/// 断言类必带溯源（I2）：`produced_by` 指向触发检索的那格用户发言。
pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    EVENT_MEMORY_RECALLED,
    (Entity::Memory, Verb::Asserted),
)];
