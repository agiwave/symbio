//! 通用服务实现层
//!
//! 各可插拔服务的**具体实现**在这里。抽象（trait）在 [`crate::symbio_core::EmbeddingService`]
//! 与 [`crate::symbio_core::vdfs`]。
//!
//! ## 子模块
//!
//! - `embedding`：[`crate::symbio_core::EmbeddingService`] 的实现
//!   （经工厂 `creator_create_object::<dyn EmbeddingService>(...)` 取用）
//! - `vdfs_service`：**基于 `VdfsProvider` 接口的集中实现**（单文件 / 目录 / 内存）
//! - `collectors`：`*Visitor` 三个契约的**内存默认实现**（写入者是全体插件）
//! - `memory`：「单文件长期记忆」的实现（work / session / agent 三插件共用一份口径）
//!
//! ## 准入判据：**被多个插件模块公用**
//!
//! > **只有多个插件模块公用的东西才进本层。只有一个模块用的，一律下沉回那个模块。**
//!
//! 这是 [ADR-023](../../../docs/DECISIONS.md)「依赖方数量」判据在本层的直接应用，
//! 与 `symbio_core` 用的是同一把尺子。**出口的每个符号都要能报出它的消费者**
//! ——报不出的就不该在出口（见下方 `pub(crate) use` 处对 `MemoryInjection` 的说明）。
//!
//! 实测对照表（**插件侧**生产消费者，不含本层自身与 core）：
//!
//! | 符号 | 消费者 | 数 |
//! |---|---|---|
//! | `MemoryFile` · `MemoryNodeSpec` · `MemorySegmentSpec` | work · session · agent | 3 |
//! | `vdfs_id_of` | agent · mcp · model · skill | 4 |
//! | `vdfs_auto_id` | mcp · model · skill | 3 |
//! | `VdfsUnpack` | agent · mcp · skill | 3 |
//! | `DirVdfs` | mcp · skill | 2 |
//! | `SingleFileVdfs` · `MemoryVdfs` | model | 1 |
//! | `VdfsPack` | agent | 1 |
//! | `vdfs_safe_segment` | session | 1 |
//! | `Default*Visitor` ×3 | 各 1 个**安装者** + 2–3 个插件的**测试** | — |
//! | `EMBEDDING_LOCAL` | local | 1 |
//!
//! **「数为 1」不等于「该下沉」** ——判据量的是**那件东西**，不是它所在的那一行：
//!
//! - `SingleFileVdfs` / `MemoryVdfs` 与 `DirVdfs` 是**同一个存储装配层的三种拓扑**，
//!   共享 `entry`（id / 段名口径）与 `pack`（整包 zip）；该层整体被 4 个插件使用。
//!   把两个拓扑单独搬去 model 会切断这份共享，并把「三种拓扑、一份磁盘布局」
//!   这个设计（见 `vdfs_service/mod.rs` 模块头）拆碎。
//! - `VdfsPack` 是 `VdfsUnpack` 的**反向**（导出 / 导入成对），后者有 3 个消费者。
//! - `vdfs_safe_segment` 与 `vdfs_id_of` / `vdfs_auto_id` 是**同一条「条目 id 安全化」
//!   口径**。让 session 自己写一份，同一个 id 就会在两个地方被安全化成两种样子。
//! - 三个 `Default*Visitor` 的**写入者是全体插件**（每个插件都经 `ctx` 的
//!   `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR` 键注册），
//!   安装它的宿主只是其中之一；另有 2–3 个插件的测试要用它当「能读回的收集器」。
//!   完整论证见 `collectors/mod.rs`。
//!
//! ## 两种接线方式，各自说清理由
//!
//! - **可替换的宿主服务**走工厂：trait 在 core、实现在本层、业务模块只 `use` trait，
//!   例如 `embedding`。这类服务的价值正是「换实现不改调用方」。
//! - **不存在第二种实现的机制底座**直接组合具体类型：`vdfs_service` 的三个实现
//!   本身就是 `VdfsProvider`（接口在 core 已定，不会换），`memory` 的三个消费方
//!   （work / session / agent）在**编译期**就知道自己要用哪种记忆——两者都不存在
//!   第二种实现，再套一层 `dyn` 工厂只是把一次构造调用换成一次字符串查表。
//!   因此业务模块**直接** `use crate::providers::{DirVdfs, SingleFileVdfs, MemoryVdfs}` /
//!   `use crate::providers::MemoryFile`。
//!
//! **为什么后两者不能改成工厂（`creator_create_object`）**——不是偏好，是机械约束：
//!
//! 1. `type ObjectConstructor = fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>`
//!    **没有参数位**，而 `DirVdfs::at(dir, plugin_id, manifest)` 与
//!    `MemoryFile::new(path, write_max, inject_max)` 的参数来自插件自己的配置，
//!    不在 `ctx` 里。
//! 2. `DirVdfs` 有 **21 个**非 trait 的 `pub fn`（`write_json` / `entries` /
//!    `import_pack` / `entry_dir` …），插件用的正是这些；工厂只能返回
//!    `dyn VdfsProvider`，那些方法一个都调不到。
//!
//! 把它们塞进工厂等于「三次 `ctx` 存取 + 一次字符串查表 + 一次 downcast」换掉一次
//! 编译期检查的直接构造——[ADR-035](../../../docs/DECISIONS.md) 决策 1 否决的就是这个。
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

// ⭐ 子目录一律**私有**（与 `providers/embedding` 的同一条约定：子模块不带
// `pub(crate)`）。每个目录自己的 `mod.rs` 是它的唯一出口，本文件再把各出口
// **逐符号平铺**给 crate 内使用——与 `symbio_core` 的「一个出口（根平铺重导出）」
// 同构：消费方一律写 `crate::providers::<符号>`，不深引 `::<目录>::<子模块>::`。
// 深引会让出口形同虚设（子模块一改名就要改所有调用点），也让「公开面」散成两套。
mod collectors;
mod embedding;
mod memory;
mod vdfs_service;

pub(crate) use collectors::{DefaultConfigurableVisitor, DefaultOptionVisitor, DefaultToolVisitor};
// `MemoryInjection` / `memory_render_segment` **不出口**：注入产物的形状与排版函数
// 只有 `MemoryFile` 自己在用（插件拿到的是 `segment()` 出来的 `String`），
// 出口只列**真有跨模块消费者**的符号——否则「公开面」里会混入没人要的东西，
// 也就无从判断这条实现到底被谁依赖。
pub(crate) use memory::{MemoryFile, MemoryNodeSpec, MemorySegmentSpec};
// `embedding` **不在这里**：它没有直接出口，两个实现经 `submit_object_creator!`
// 注册，业务模块用 `creator_create_object::<dyn EmbeddingService>(EMBEDDING_*, ctx)` 取。
pub(crate) use vdfs_service::{
    auto_id as vdfs_auto_id, id_of as vdfs_id_of, safe_segment as vdfs_safe_segment, DirVdfs,
    MemoryVdfs, SingleFileVdfs, VdfsPack, VdfsUnpack,
};
