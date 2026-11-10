//! 目标自生成能力包 —— **「欲」的表达与它的升格读数**（名册 #31，DE-026 §2.3 / DE-042）。
//!
//! 本 crate 是「读数类能力搬出控制面」的第一个落点，验证的是 `D-281` 开的那一格：
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | 只有 `conation.expressed`，落在 `conation × opened`；#36 / #50 / #51 的零写方声明面已按 `D-411` 撤回，设计行留名册 | 写闸按声明校验名字与格位；名字进 `shared_events` 是因为写方在会话（跨插件） |
//! | 视图（`ViewDecl` + 报表登记） | `conation.intents` 折出意图流 + 两个计数 + 采纳率 | 任何按名字读的插件（D-221） |
//! | 多片合并（`ViewDecl::agg`） | N 份单片读数 → 实例级的采纳率 | 实例级聚合列的读方（D-281） |
//! | 债（`DebtSource`） | 「这一条欲该不该升格」的三个 early return（未 enabled / 未授权 / 已声明） | 环的计划层（D-386） |
//! | 端口（`PortProvider`） | 执行体自判授权、把「欲」升格成 `task.opened` | 环的派发层（`D-385`） |
//!
//! 没有工具、没有路由、没有行为——`kernel` / `sdk` / `loop` 一行都不动；
//! `session` 只做**删除**：触发、健康与「欲」照旧入格，判决与升格不再由它读 / 写（`D-386`）。
//!
//! ## 为什么它有 `Plugin` 实现却没有工具与路由
//!
//! 四条扩展面都走链接期收集，实例不参与任何判断；实例存在只为**挂载**——装配清单按插件
//! 目录挂载，停用插件 ⇒ 实例不构造。`route` 恒 `NotFound`、`traverse` 只申报债行与端口
//! 提供者行，两者都不是占位符而是答案（与 `symbio-plugin-task` 同一形状、同一理由）。
//!
//! 写入由会话的自主触发完成（会话存储所有者的写窗口），升格由本包的执行体在环里完成。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api`、`serde` 与 `serde_json`。HS-001 判的
//! `kernel` / `sdk` / `deploy` 一个都不列，也不认识任何兄弟插件。

mod autonomy;
mod config;
mod debt;
mod decl;
mod plugin;
mod port;
