//! retrieval 插件 —— **可选**的检索者（S06）：登记检索 Actor 行 + 一条只读召回路由。
//!
//! ## 它做什么
//!
//! 1. **登记** S06 的"检索者"一行 —— `session.retrieval`
//!    （`{ pattern: translator, budget_ms: 500, scope: root }`）。这是 B4 的**第一个
//!    真消费者**：B3 的 `decider` 只是"能登记"的演示（它不读事实、不产出事实），
//!    检索者不同——它读 `Fact`、跑投影，把 B1 / B2 / B3 三者串成一条链；
//! 2. 提供 `retrieval/list` 路由：**跑一次 `memory.recall` 投影**，返回召回候选。
//!
//! ## 它与前三个可选插件（`fact_log` / `projection` / `actor`）的差别
//!
//! | 插件 | 提供什么 | 消费什么机制 |
//! |---|---|---|
//! | `fact_log` | 事实派生 | B1（`FactSource`） |
//! | `projection` | 投影内省口 | B2（`projection_run`） |
//! | `actor` | Actor 内省口 + 演示行 | B3（`actor_register`） |
//! | **`retrieval`** | **真正的召回** | **B1 + B2 + B3 三者** |
//!
//! ## 三条设计边界
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **可选**：停用 / 未装配 = 系统照常运行（J2 平凡值） | 它提供的是**召回能力**，不是地基。不在 `ASSEMBLY_SUB_AGENT_PLUGINS` 里 |
//! | **只读**：只派生事实、跑投影、登记一行，不写任何域 | 检索是**读视图 → 产出事实**；本轮交付读的那半 |
//! | **零插件依赖**：只经 `symbio_core` 的 fact / projection / actor API，不 import 任何兄弟插件 | 插件独立原则。它**不认识** `session` / `fact_log` |
//!
//! ## 为什么不复用 `fact_log` 的事实源
//!
//! `fact_log` 把事实登记进 `CapabilityVisitor`、消费方经 `list_fact_sources` 取用。
//! 但那条路要求 `retrieval` **认识 `CAPABILITY_VISITOR` 的登记顺序**——而事实源登记
//! 发生在 `traverse` 期，与 `retrieval` 的 `route` 调用**没有时序保证**。
//! 更重要的是：**依赖兄弟插件的运行时产物 = 隐式插件依赖**。
//! 因此本插件沿用 `fact_log` 自己的先例——**只依赖磁盘上的公开布局**
//! （`<root>/session/<id>/messages.json` 与 `MEMORY.md` 是公开约定，
//! 见 `docs/design/vdfs.md`），**就地**派生。于是两者是同一份约定的两个独立消费者。
//!
//! ## 文件
//!
//! - [`derive`]：从磁盘派生事实（含 `memory.*` 格子）的**纯函数**——本插件的全部机制
//! - [`plugin`]：插件本体（`Plugin` 实现 + 检索行登记 + `list` 路由）

mod derive;
mod plugin;
