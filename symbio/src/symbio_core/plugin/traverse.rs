//! `traverse` 的两个协议端点 —— `Plugin::traverse(path, ctx)` 的合法取值
//!
//! `traverse` 是一次**广播**：宿主对父插件调一次，所有插件在同一契约下按
//! `ctx[PATH] == <端点>` 判定自己要不要贡献，宿主在同一次广播里收齐。
//!
//! | 端点 | 收集什么 | 产物 | 消费链路 |
//! |---|---|---|---|
//! | [`TRAVERSE_AVAILABLE_TOOLS`] | 可调用对象（`Capability`）+ 模型服务 + 系统提示词 + VDFS provider | `CapabilityMeta` / provider 实例 | 会话链路（LLM 工具调用） |
//! | [`TRAVERSE_AVAILABLE_OPTIONS`] | 可展示的数据节点（会话配置表单字段） | `DetailField` | 前端链路（表单 / 详情渲染） |
//!
//! ## 它们**不是**插件路径
//!
//! 两个值都是端点名，容器不会拿它们去找子插件（对比 [`super::route`] 的
//! `<插件目录名>/<子路径>`）。值（`"available_tools"` / `"available_options"`）
//! 是插件与调用方之间的字面量约定，跨进程可见，**不可改**；标识符刻意**不带**
//! `ROUTE_` 前缀，正是为了与路由地址区分开——这条边界由
//! `scripts/core-naming-audit.mjs` 守住。
//!
//! ## 为什么住 `plugin` 域
//!
//! 它们是 [`Plugin::traverse`](crate::symbio_core::Plugin::traverse) 的入参取值，
//! 与 `Plugin` trait 是**同一份契约**。此前两者分居两处（一个在 crate 根、一个在
//! `capability` 域），同一份协议的两个端点看不出是同一份协议；现在同处一个文件。
//!
//! `capability` 域只是 [`TRAVERSE_AVAILABLE_TOOLS`] 那条通道的**产出方之一**
//! （它贡献工具与选项字段），它不拥有这个端点。

/// `traverse` 端点：收集可用工具（LLM 可见能力）、模型服务、系统提示词与 VDFS provider
pub const TRAVERSE_AVAILABLE_TOOLS: &str = "available_tools";

/// `traverse` 端点：收集可用选项（会话配置表单字段，产物 `DetailField`）
pub const TRAVERSE_AVAILABLE_OPTIONS: &str = "available_options";
