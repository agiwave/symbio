//! 事实信封域 —— 全系统**可观测事实**的统一只读公共面。
//!
//! ## 这个域解决什么
//!
//! 现行体系里有**三套并行的事件方言**，互不相识：
//!
//! | 方言 | 住哪 | 谁知道它 |
//! |---|---|---|
//! | `ChatMessage` | 会话的 `messages.json` | 只有 `session` |
//! | `PluginFrame` | 插件间传输 | 容器与各插件 |
//! | `VdfsChange` | 资源变更 | VDFS 链路 |
//!
//! 后果不是"不统一不好看"，而是**新能力无处依附**：一个检索者想知道"哪些事实发生过"，
//! 它只能去读 `messages.json`（那是 `session` 的私有布局），于是新能力与 `session`
//! **产生代码依赖**——这违反「插件之间不可相互引用」。
//!
//! 本域提供**一个**信封 [`Fact`]，把上述方言**读侧**归一。于是新增能力只需要知道
//! 「自己读哪些 [`FactKind`]、写哪些 [`FactKind`]」，不需要知道任何既有模块。
//!
//! ## 关键边界：只读，且不是第二份真相
//!
//! - [`Fact`] **没有写入口**（不提供 `append` / `store`）——写入仍走各自原有路径
//!   （会话写 `SessionStore`、资源写 `VdfsProvider`）。因此它**不可能**与源数据分叉：
//!   它是**派生**的，不是**并行存储**的。
//! - 派生由 [`FactSource`] 实现方负责，实现方**通常就是那个数据的 owner**
//!   （例如会话事实由 `session` 插件派生）。派生是**纯函数式**的：给定同一份源数据，
//!   永远得到同一串 `Fact`（`seq` 顺序确定），因此可双跑比对。
//!
//! ## 与 v2 目标态的关系
//!
//! 本域对应 v2「事件信封 + 机制表」的落地形态，但**不走"重写存储为 Event Log"那条路**
//! （见 `docs/plan/06-落地桥接方案.md` §2.1）：
//!
//! | v2 概念 | 本域 |
//! |---|---|
//! | `Store`（原语，append-only 事实源） | **不改变**现有存储；`Fact` 是它们的只读投影 |
//! | `Event`（共享契约） | [`Fact`] + [`FactKind`] |
//! | `Principal`（身份） | [`FactPrincipal`]（字符串身份，数据无限） |
//! | I2 无溯源不声明 | [`Fact::caused_by`]（可选溯源，指向更早的 `seq`） |
//! | 事件网格 10 实体 × 5 动词 | [`FactKind`] 的穷举取值（[`FactKind::ALL`] 即机制表的一行） |
//!
//! ## 命名
//!
//! 本域前缀是 `fact`（类型 `Fact*` / 函数 `fact_` / 常量 `FACT_`），见
//! [`../README.md`](../README.md) §1.2 的域前缀表。

mod envelope;
mod kind;

pub use envelope::{Fact, FactError, FactPrincipal, FactSource, FACT_NONE_SEQ};
pub use kind::FactKind;

#[cfg(test)]
mod tests;
