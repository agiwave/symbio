//! VDFS 存储装配层——**基于 `VdfsProvider` 接口的集中实现**
//!
//! 取代旧的 `storage_service`（`StorageService` + `EntityStore` + `FileEntityStore`）。
//! 旧那一层的问题不是「多了一层」，而是**它讲的不是 VDFS 的话**：磁盘资源用一套
//! 私有 trait（`list_entities` / `read_entity` / `write_entity`）表达，再由每个插件
//! 手翻成 `VdfsNode` / `VdfsContent`，于是「一类资源 = 一份存储抽象 + 一份翻译」。
//!
//! 这里反过来：**存储实现直接就是 `VdfsProvider`**。三种拓扑、一份磁盘布局：
//!
//! | 实现 | 一个条目 = | 条目内部 | 适用 |
//!|---|---|---|---|
//!| [`SingleFileVdfs`] | 一份主文件 | **不外露**（叶子） | Model Provider |
//!| [`DirVdfs`] | 一个目录 | 可下钻浏览 | Skill / MCP Server |
//!| [`MemoryVdfs`] | 内存一条记录 | 无 | 运行期注册表 / 磁盘镜像 |
//!
//! 三者的磁盘布局**完全一致**（`<类别根>/<id>/<manifest>`），
//! 差别只在**访问拓扑**，因此换型不动数据。
//!
//! ## 为什么放在 providers 而不是 symbio_core
//!
//! `symbio_core::vdfs` 是**纯接口**（只依赖 std / serde / async_trait，
//! 可原样抽出为独立 crate）；带 tokio IO 与 homedir 的**实现**属于宿主基础设施，
//! 归本层。插件经 [`crate::providers`] 的出口取用它们（`DirVdfs` / `SingleFileVdfs` /
//! `MemoryVdfs`），**不深引**本目录的子模块。
//!
//! ## 与「废除实体机制」的关系
//!
//! 本模块**不定义任何新抽象**——没有 trait、没有注册表、没有适配器。每个资源仍然
//! 自己 `impl VdfsProvider`；本模块只是把那些 `impl` 里重复的**落盘 plumbing**
//! （id 安全化、条目寻址、原子写、mtime、整包 zip、变更广播）收成一处。
//! 差异（标题、状态、`ext`、`schema`、写前校验、写后内存同步）仍留在各插件，
//! 由它们在调用点以普通 Rust 参数传入，不经过任何通用钩子。

// ⭐ 子模块一律私有（与 `providers/embedding` 同一约定）：**本文件是唯一出口**，
// 消费方按出口取符号，不深引子树（深引会让出口形同虚设——改内部结构就要改所有调用点）。
// 四条线各自说清为什么要外露：
mod dir;
mod entry;
mod memory;
mod pack;
mod single_file;

/// 三种访问拓扑（一个条目 = 一份主文件 / 一个目录 / 内存一条记录）
pub(crate) use dir::DirVdfs;
pub(crate) use memory::MemoryVdfs;
pub(crate) use single_file::SingleFileVdfs;

/// 整包导入导出：zip 落盘 + base64 编解码。插件的节点动作 `import` / `export`
/// 直接要这两个类型，故进出口。
pub(crate) use pack::{VdfsPack, VdfsUnpack};

/// 条目 id / 段名规则：地址解析（`id_of`）与落盘（`safe_segment`）、以及
/// 「无名字新建条目」的 id 生成（`auto_id`）是**多个插件共用的同一条口径**
/// ——各插件自己写一份就会出现「同一个 id 在两个插件里被安全化成两种样子」。
/// 名字太泛（`id_of` 单独读不出归属），故经 [`crate::providers`] 平铺时带 `vdfs_` 前缀。
pub(crate) use entry::{auto_id, id_of, safe_segment};
