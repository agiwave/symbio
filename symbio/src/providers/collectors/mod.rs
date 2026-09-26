//! 收集器的默认实现 —— 三个 `*Visitor` 契约的内存版
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 三个 `*Visitor` **trait** 是契约（住 `symbio_core::capability`）；它们的**默认实现**
//! 是「不存在第二种实现的机制底座」，因此按 [`crate::providers`] 的第二种接线方式
//! **直接组合具体类型，不套 `dyn` 工厂**——再套一层 `creator_create_object` 只是把一次
//! 构造换成一次字符串查表（与 `vdfs_service` 同款判断；ADR-035 决策 1.1「≥2 实现才
//! provider 化」因此**不适用于这里**）。
//!
//! ## 为什么不是「归安装它的那个插件」
//!
//! 收集器的**写入者是全体插件**（经 `ctx` 的 `CAPABILITY_VISITOR` / `OPTION_VISITOR` /
//! `CONFIGURABLE_VISITOR` 键），安装它的宿主只是**其中之一**——它是跨插件的共享实现，
//! 不是任何插件的内部物。
//!
//! 实测证据：`vdfs` / `agent` / `work` 三个插件的测试都需要一个能读回的收集器。若它住
//! `session`，这三个测试就要跨插件引用 session 的类型，违反「插件之间不直接相互引用」。
//! 住本层则任何模块（含各插件测试）都能用，**不构成插件间依赖**。
//!
//! ## 契约与实现的分工
//!
//! | 部分 | 住哪 |
//! |---|---|
//! | `*Visitor` trait · 上下文键（`*_VISITOR`）· 遍历端点字面量 | `symbio_core::capability` |
//! | 收集管线（`collect_capabilities` / `collect_options`） | 各自的宿主（session / composite） |
//! | **内存版收集器**（本目录） | `providers/` |
//!
//! 只有「无策略的内存实现」放这里；排序 / 去重之外的任何策略都归调用方。

// ⭐ 子模块一律私有（与 `providers/embedding` 同一约定）：**本文件是唯一出口**。
mod configurable_visitor;
mod option_visitor;
mod tool_visitor;

pub(crate) use configurable_visitor::DefaultConfigurableVisitor;
pub(crate) use option_visitor::DefaultOptionVisitor;
pub(crate) use tool_visitor::DefaultToolVisitor;
