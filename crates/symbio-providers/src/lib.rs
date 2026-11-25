//! 通用服务实现层
//!
//! 各可插拔服务的**具体实现**在这里。抽象（trait）在 `crate::symbio_platform::sdk::embedding::EmbeddingService`
//! 与 `crate::symbio_platform::kernel::vdfs`。
//!
//! ## 子模块
//!
//! - `embedding`：`crate::symbio_platform::sdk::embedding::EmbeddingService` 的实现
//!   （经工厂 `creator_create_object::<dyn EmbeddingService>(...)` 取用）
//! - `vdfs_service`：**基于 `VdfsProvider` 接口的集中实现**（单文件 / 目录 / 内存）
//! - `collectors`：`*Visitor` 三个契约的**内存默认实现**（写入者是全体插件）
//!
//! 「单文件长期记忆」此前也住在这里，唯一生产消费方 `memory` 插件已按 D-534 把它
//! 收回自己的私有模块（`symbio-plugin-memory/src/memory_file.rs`），本层不再出口。
//!
//! ## 准入判据：**被多个插件模块公用**
//!
//! > **只有多个插件模块公用的东西才进本层。只有一个模块用的，一律下沉回那个模块。**
//!
//! 这是 D-023「依赖方数量」判据在本层的直接应用，
//! 与 `symbio-plugin-api` 用的是同一把尺子。**出口的每个符号都要能报出它的消费者**
//! ——报不出的就不该在出口（见下方 `pub use` 处对 `MemoryInjection` 的说明）。
//! 实测消费者表由 `RE-006-当前事实表` §6.2 从生产源码生成，本文件不再手抄；
//! 出口变动只需改下方 `pub use`，生成事实随之变化。
//!
//! ## 两种接线方式，各自说清理由
//!
//! - **可替换的宿主服务**走工厂：trait 在 core、实现在本层、业务模块只 `use` trait，
//!   例如 `embedding`。这类服务的价值正是「换实现不改调用方」。
//! - **不存在第二种实现的机制底座**直接组合具体类型：`vdfs_service` 的三个实现
//!   本身就是 `VdfsProvider`（接口在 core 已定，不会换）；业务模块在编译期就知道
//!   自己要用哪种拓扑，再套一层 `dyn` 工厂只是把一次构造调用换成一次字符串查表。
//!   因此业务模块**直接** `use crate::providers::{DirVdfs, SingleFileVdfs, MemoryVdfs}`。
//!
//! **为什么直接组合的底座不能改成工厂（`creator_create_object`）**——不是偏好，是机械约束：
//!
//! 1. `type ObjectConstructor = fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>`
//!    **没有参数位**，而 `DirVdfs::at(dir, plugin_id, manifest)` 的参数来自插件自己的
//!    配置，不在 `ctx` 里。
//! 2. `DirVdfs` 有 **21 个**非 trait 的 `pub fn`（`write_json` / `entries` /
//!    `import_pack` / `entry_dir` …），插件用的正是这些；工厂只能返回
//!    `dyn VdfsProvider`，那些方法一个都调不到。
//!
//! 把它们塞进工厂等于「三次 `ctx` 存取 + 一次字符串查表 + 一次 downcast」换掉一次
//! 编译期检查的直接构造——D-035 决策 1 否决的就是这个。
//!
//! ## 磁盘布局（统一约定）
//!
//! ```text
//! ~/.symbio/
//! ├── PLUGIN.yml                               # 系统级插件（home）自身配置
//! ├── <插件>/                                  # ⭐ 一个插件 = 一个目录（配置 + 数据同处）
//! │   ├── PLUGIN.yml                           # 该插件自己的配置
//! │   └── <id>/<主文件>                         # model/<id>/provider.json · mcp/<id>/server.json
//! └── session/<id>/session.json               # Session（自有 store，不经 vdfs_service）
//! ```
//!
//! **重要**：一个插件 = 系统根下的一个目录，**便于通过遍历
//! `~/.symbio/` 即可知道加载了哪些插件**。
//!
//! workdir 不在本层持有：它始终由前端在每个请求的 `ctx.WORKDIR` 中显式传递。

// 实现源码随所有者迁入本 crate 的私有模块；出口仍是原来的 `symbio_providers::`
// 平铺面，模块路径只保留给需要深引的装配代码。
pub mod collectors;
#[cfg(feature = "embedding")]
pub mod embedding;
pub mod vdfs_service;

pub use collectors::{
    approval_gate, new_registry, register_tool, register_tool_with_async_facts,
    register_tool_with_facts, AsyncFactsOf, FactsOf, RiskOf,
};
// `DefaultToolVisitor` 是测试面句柄；生产装配一律经 `new_registry()`。
pub use collectors::DefaultToolVisitor;
// `embedding` 实例经 `submit_object_creator!` 注册，调用方只取「为什么不可用」。
#[cfg(feature = "embedding")]
pub use embedding::unavailable_reason as embedding_unavailable_reason;
pub use vdfs_service::{
    agent_home_root, attach_round_providers, attach_scope_providers, auto_id as vdfs_auto_id,
    id_of as vdfs_id_of, safe_segment as vdfs_safe_segment, DirVdfs, MemoryVdfs, ScopeDirVdfs,
    SingleFileVdfs, VdfsPack, VdfsUnpack,
};
