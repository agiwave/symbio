//! Symbio SDK：**插件可以依赖的宿主机制层**（DE-023 D-258）
//!
//! ## 这一层治什么
//!
//! `symbio-plugin-api` 的门面按来源分臂之后（D-255），机制臂一度还有 115 条——每一条都指向
//! 宿主的一个子系统（`PluginDir` 的配置 IO、`EventBus` 的订阅表、`capability_invoke`
//! 的调用栈、`principal` 的授权表）。那些子系统**不认容器概念**：它们只碰内核的词汇
//! （`PluginInvokeRequest` / `Event` / `VdfsChange` / `Store`）与自己的表，却和控制面住在一起，
//! 于是「插件要用一条机制」和「插件要认识整个宿主」被绑成同一件事。
//!
//! 本 crate 收的就是这批机制，只依赖 `symbio-kernel`：插件门面 = 语法（kernel）+ 机制（sdk）。
//! D-279 之后它同时是**控制面原来那层的落点**——旧 core 整棵并入本 crate，控制面
//! （`loop` / `session`）改为直接依赖 `symbio-kernel` + `symbio-sdk`，架构 crate 于是只剩两个。
//!
//! ## 两层判据（照 D-256 / D-257 / D-279 的口径）
//!
//! | 层 | 收什么 | 判据 |
//! |---|---|---|
//! | `symbio-kernel` | **语法类型**（形状、闭集、线格式常量）与**冻结机制原语**（信封解包 / 出口与中止 / 存储基线，逐条见 D-398） | **不含任何业务名、owner 归属、能力名、端口名常量**（CS-010 可执行；四类原语的清单靠人工审，审的是「能不能在不认识任何业务主语的前提下说清它的合同」） |
//! | `symbio-sdk`（本 crate） | 宿主机制与策略：配置 IO、事件总线、地址装配、授权判定、日志、投影与装配 | 有实现；机制部分**不认容器概念**，控制面部分认识（DE-026 的影响类） |
//!
//! 「认不认识容器概念」用来判**一条符号该给插件看还是只给控制面看**，不是用来判它住哪个 crate
//! （D-276 §2：crate 表达依赖方向，模块表达归属）：`vdfs_watch_changes` 有实现，但它只对地址
//! 与变更说话，所以它在门面上；`turnstate` 也是纯类型，但它描述的是**一轮**，所以它只在
//! 控制面可见（由控制面持有，D-197）。
//!
//! ## 根为什么平铺内核词汇
//!
//! 本 crate 的模块沿用 `use super::*;` 取词汇（那是它们搬过来之前的写法），而它们要的
//! 词汇就是内核的封闭语法。根对内核做一次**私有** `use symbio_kernel::*`：子模块取得到，
//! crate 外写不出 `symbio_sdk::<内核符号>`——内核符号在 crate 外只剩 `symbio_kernel::…`
//! 一条路径（DE-018 的 D-419）。各域模块同样只私有转引内核词汇，公开面只留自己的实现。
//!
//! `inventory` 同样出根：`submit_object_creator!` 宏里写的是 `$crate::inventory::submit!`，
//! 而 `$crate` 展开成本 crate——宏的使用方不必依赖 `inventory`。

pub use inventory;
use symbio_kernel::*;

// `submit_declarations!` 的宏体写的是 `$crate::DeclarationsDecl`：收集类型与本 crate 同住，
// 使用方不必认识 `symbio_sdk` 的模块路径（与 `submit_object_creator!` 同形）。
pub use declarations::DeclarationsDecl;

pub mod actors;
pub mod adapters;
pub mod agent;
pub mod authz;
pub mod capability;
pub mod chat;
pub mod clock;
pub mod creator;
pub mod declarations;
pub mod embedding;
pub mod event_bus;
pub mod governance;
pub mod invariants;
pub mod keys;
pub mod llm;
pub mod logger;
pub mod plugin;
pub mod port_stamp;
pub mod projection;
pub mod text;
pub mod turn;
pub mod vdfs;

/// 用例自己交的事件拼写与三张词表（DE-024 D-203 / D-205 / D-206）。
///
/// 只在 `cfg(test)` 或 `test-fixtures` feature 下存在：生产拼写来自各 owner 的清单
/// 生成物，本模块一份都不参与生产。它随 `projection` 搬来（D-259）——`projection` 的
/// 用例与仍住 core 的 `actors` / `governance` 用例共用同一份拼写，两份真源会让
/// 「同一性质两份结论」重演（那是 D-188 抽 `invariants::kernel` 时踩过的坑）。
#[cfg(any(test, feature = "test-fixtures"))]
pub mod test_fixtures;
