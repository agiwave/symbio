//! VDFS 挂载族的**读面**——只剩根（DE-025 §3 的收窄版）。
//!
//! ## 一族只剩一形：根（单槽）
//!
//! 根是「谁服务 `<根>` 这棵子树」这个**装配时的安排**，它不是某个插件的目录，
//! 也不可能由调用方命名。写侧是容器在 `traverse` 广播里 `put(CapabilityRow::mount_root(..))`，
//! 读侧只有 [`capability_vdfs_root`] 一个入口。
//!
//! 根行带**根位**（[`CapabilityRow::active`]），单槽移交因此与模型族的生效位同一条规则
//! （`put` 里那一段），不需要第二套机制。
//!
//! ## 根为什么是**单槽**，以及谁被挡在外面
//!
//! 根归系统 Agent：子智能体子树里也有容器（它会把自己登记成根），其注册经
//! `crates/symbio-plugin-agent/src/host/scope.rs::SubAgentVisitor` **丢弃**——否则子树会劫持整个 `<根>`，
//! 系统侧全部挂载点瞬间消失（判据 = 该文件的单测 + e2e `t39`）。
//!
//! ## 挂载清单（非根的挂载行）为什么不存在
//!
//! 第二形本来是：容器为每个交得出 VDFS 视图的子插件派生一行
//! `CapabilityRow::mount(目录名, provider)`，配两个读口 `capability_mounts` /
//! `capability_mount`。那两个读口**从来没有生产消费方**（唯一读点是单测）——
//! 「哪个目录由谁提供」这个问题已经被两处更强的地方回答：
//!
//! - `CompositeVdfs` 在装配期就把子插件的 provider **聚合**成组合视图（系统链路的唯一入口）；
//! - `Plugin::get_vfs_provider` 是「这个插件交不交得出视图」的唯一判据（容器聚合时问的也是它）。
//!
//! 行表是这两者的**第三份复述**：写得进去、读不出来。按 [DE-015 §D-144](../../../../docs/003-design/015-义务环.md "DE-015-义务环")
//! 的裁决，读口与派生写一起退场——留着它，每个读代码的人都要重新判一次这是不是活约定。
//! 真要「按可见域给 LLM 裁剪资源」，消费方接在**聚合视图**上（那才是资源的真相），
//! 不是再接一张行表；而裁剪本身属于地址解析（`hidden` 过滤 / `resolve` 命中范围），
//! 不是拿一份清单做集合减法。

use super::{family, Registry};
use std::sync::Arc;
use symbio_kernel::vdfs::VdfsProvider;

/// 取 VDFS 根 provider（**服务 `<根>` 的那一个**）。
///
/// 无容器注册时为 `None`（等价于「系统里没有资源」）——调用方给空根降级
/// （`crates/symbio-plugin-vdfs/src/host.rs::root_of`），不在这里替它造一个。
pub async fn capability_vdfs_root(visitor: &dyn Registry) -> Option<Arc<dyn VdfsProvider>> {
    visitor
        .list(family::MOUNT)
        .await
        .into_iter()
        .find(|row| row.active())
        .and_then(|row| row.vdfs().cloned())
}
