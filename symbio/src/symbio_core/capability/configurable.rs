//! 可配置声明收集机制（跨插件共享设施）
//!
//! ## 与能力 / 选项收集机制的关系
//!
//! 三者共用**同一套收集机制**（`traverse` 广播 + 上下文键 + 失败降级），
//! 区别只在产物：
//!
//! | 通道 | 收集器 | 产物 |
//! |---|---|---|
//! | 能力 | [`crate::symbio_core::CapabilityVisitor`] | 可调用对象（工具 / 模型服务 / 提示词） |
//! | 选项 | [`crate::symbio_core::OptionVisitor`] | 可展示的数据节点（会话输入区的可选项） |
//! | 可配置 | [`ConfigurableVisitor`] | 「设置页里该怎么列这一份配置」的**条目** |
//!
//! ## 为什么收集的是「一个条目」而不是「一个 provider」
//!
//! 声明说的是「**我有**配置、长这样」，不是「配置由我代管」——配置的读写仍归
//! **拥有者**（插件配置 = 插件目录里的一个文件，见
//! [`PluginConfigFile`]）。所以声明里装的就是设置页列这一项所需的
//! 全部信息，而它们**只有拥有者说得出来**：
//!
//! - **地址**：配置文档的真实地址 `<目录名>/PLUGIN.yml`——设置页的条目指向它，
//!   读写因此仍落在拥有者自己的文件上，同一份配置只有一个地址；
//! - **标题**：人读名（「会话设置」）。光看磁盘上的 `PLUGIN.yml` 只知道目录名，
//!   不知道它该显示成什么，这正是本通道存在的理由；
//! - **呈现扩展名与表单定义**：设置页据此把条目渲染成**同一个表单**——定义只有
//!   一份来源（拥有者的 [`PluginConfigFile`]），
//!   列表只是把它转述出去。
//!
//! ## 为什么产物是 [`VdfsItem`] 而不是 [`VdfsNode`](crate::symbio_core::VdfsNode)
//!
//! 因为**地址在这里不可推导**。设置页的条目指向 `<目录名>/PLUGIN.yml`——那是一个
//! **跨挂载点**的地址（文件在插件自己的目录里，条目却列在设置页这棵子树下）。
//! 分发层回填地址的规则是 `<父地址>/<name>`，在这里会算成
//! `plugin_manager/<目录名>`——一个根本不存在的位置。所以拥有者必须**连地址一起**交出来，
//! 而地址属于「这一份列表」，不属于节点本身（节点是纯自述）。这就是 [`VdfsNode`](crate::symbio_core::VdfsNode)
//! 不带 `path`、而 [`VdfsItem`] 带的理由；本文件是这条边界的**唯一现存实例**。
//!
//! 图标不在这里：VDFS 不下发图标，前端按 `kind:名字` 查自己的 UI 映射表
//! （`registry/vdfsIcons.ts`，`plugin_manager:<目录名>` 已登记）。声明只带数据，
//! 不带 SVG。
//!
//! ## 注册时机
//!
//! 与 `register_vdfs_provider` 完全相同：插件在自己的
//! [`TRAVERSE_AVAILABLE_TOOLS`](crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS)
//! 分支里顺带声明，一次广播同时喂饱两条链路。插件侧只有一个入口
//! [`capability_announce_configurable`]，条目形状在那里统一构造，各插件不必各写一遍。
//!
//! 与 VDFS provider 的**逐子插件独立收集器**不同，本通道用**共享收集器**：
//! 声明自带目录名，不存在归属歧义，所以容器可以让所有子插件注册进同一个实例。

use crate::symbio_core::VdfsItem;
use crate::symbio_core::{PluginConfigFile, PLUGIN_FILE};
use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt, CONFIGURABLE_VISITOR};
use async_trait::async_trait;
use std::sync::Arc;

/// 可配置声明收集器 —— 各插件在 `traverse` 中把「设置页该怎么列我的配置」交出来。
///
/// 语义与 [`crate::symbio_core::OptionVisitor`] 一致：按条目名去重、后者覆盖
/// （保留先注册的槽位），列表按注册顺序。
#[async_trait]
pub trait ConfigurableVisitor: Send + Sync + 'static {
    /// 声明一条（同条目名覆盖）
    async fn register_configurable(&self, item: VdfsItem);

    /// 列出已声明的条目（按注册顺序）
    async fn list_configurables(&self) -> Vec<VdfsItem>;
}

/// 在 `traverse` 里声明「本插件有一份配置文档」——**插件侧的唯一入口**。
///
/// 传入插件自己的 [`PluginConfigFile`]，条目形状由本函数统一构造（见 [`capability_entry_of`]）。
/// 收集器不存在时静默跳过：本通道是**增益**，没有它插件照常工作，
/// 只是设置页列不出它。收集器由容器在广播前放进 `ctx`，见
/// `plugins/composite/vdfs.rs::children_of`。
pub async fn capability_announce_configurable(
    ctx: &Arc<dyn PluginInvokeRequest>,
    config: &PluginConfigFile,
) {
    if let Some(v) = ctx.get(CONFIGURABLE_VISITOR) {
        v.register_configurable(capability_entry_of(config)).await;
    }
}

/// 「设置页条目」：把一份插件配置文档转成设置列表里的一项。
///
/// 与 [`PluginConfigFile::node`] 的区别只有**名字与地址**：
///
/// - `node()` 是「本插件目录里的那个文件」——名字 `PLUGIN.yml`，地址由分发层按
///   `<父地址>/<name>` 回填；
/// - 本函数给的是「设置列表里的一项」——名字用**目录名**（列表内唯一，前端按它
///   查图标 `plugin_manager:<目录名>`），地址用**真实地址** `<目录名>/PLUGIN.yml`，
///   必须显式带着，因为它跨挂载点、推不出来（见模块文档）。
///
/// 其余（标题 / 呈现扩展名 / 表单定义 / 访问位）直接取配置文档自己的节点视图，
/// 因此定义只有一份来源。`kind` 留空由**消费者**按自己所在的场景覆盖
/// （设置页填 `plugin_manager`）——同一条声明换个地方列，场景标签就该换。
pub fn capability_entry_of(config: &PluginConfigFile) -> VdfsItem {
    let dir = config.dir().name();
    let mut n = config.node();
    n.name = dir.to_string();
    VdfsItem::new(n).with_path(format!("{dir}/{PLUGIN_FILE}"))
}

#[cfg(test)]
#[path = "configurable.test.rs"]
mod tests;
