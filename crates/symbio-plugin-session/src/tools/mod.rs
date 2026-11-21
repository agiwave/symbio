//! 会话插件对外发布的**工具面**。
//!
//! - [`subagent`]：`agent_run`（子智能体委托）——新建 / 续会话 / 恢复委托会话。
//!   从 子智能体插件迁来：新建会话与新建会话的子会话都是会话域的事，委托是会话级
//!   原语（归属裁决见 `docs/003-design/011-Agent目录规范.md` 的关键决策）；
//! - [`behavior`]：`agent_run` 的效果与前置判断（委托 = 跨主体立约；淘汰闸管辖
//!   `assign.work`），经 `ToolBehavior` 交给写口。

pub(crate) mod behavior;
pub(crate) mod subagent;
