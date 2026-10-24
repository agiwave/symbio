//! VDFS 挂载族的**读写协议**——表（`Registry` 的 `put` / `get` / `list`）
//! 之上的挂载专用词法（AR-025 §5 步 4d）。
//!
//! ## 一族两形：挂载点（多行）与根（单槽）
//!
//! 两者都是「目录名 → provider」，差别只在**根不可能由调用方命名**：它不是某个插件的
//! 目录，而是「谁服务 `<根>` 这棵子树」这个**装配时的安排**。于是：
//!
//! | | 写 | 读 |
//! |---|---|---|
//! | 挂载点 | 容器按 `Plugin::get_vfs_provider` 派生后 `put(CapabilityRow::mount(实例名, provider))` | [`capability_mounts`] / [`capability_mount`] |
//! | 根 | `put(CapabilityRow::mount_root(provider))` | [`capability_vdfs_root`] |
//!
//! 根行带**根位**（[`CapabilityRow::active`]），单槽移交因此与模型族的生效位同一条规则
//! （`put` 里那一段），不需要第二套机制。
//!
//! ## 根为什么是**单槽**，以及谁被挡在外面
//!
//! 根归系统 Agent：子智能体子树里也有容器（它会把自己登记成根），其注册经
//! `plugins/agent/host/scope.rs::SubAgentVisitor` **丢弃**——否则子树会劫持整个 `<根>`，
//! 系统侧全部挂载点瞬间消失（判据 = 该文件的单测 + e2e `t39`）。
//!
//! ## 挂载清单：读口的消费方待接，落点是一条**带验收**的接线
//!
//! [`capability_mounts`] / [`capability_mount`] 当前**没有生产消费方**（读侧只有单测），
//! 而 [`capability_vdfs_root`] 有（`vdfs/host.rs::root_of`）——同一族协议只有一半在用。
//!
//! 保留的理由与验收写在 [DE-015 §D-144](../../../../docs/003-design/015-义务环.md "DE-015-义务环")：
//! 挂载族**不进环**（装配期声明，没有跨步等待方），但读口不删——删掉它等于让这一族只剩
//! `root` 一条查询路径，而将来给 LLM 按作用域 / 白名单裁剪可见资源时，消费方就接在这里。
//! 子智能体的注册经作用域代理在这些键上加 `agent/<id>/` 前缀。
//!
//! **这条「保留」不再靠一句「以后再说」**：D-144 把它的终态写成两条可核的验收——
//! `capability_mounts` 有一个生产读点、且有一条「不同可见域看到不同挂载集」的用例；
//! 到期不做则删。原先记的落点（S3 第 7–8 步的 `sees_of` / `vis_scope`）住在一份**已归档**
//! 的落地计划里，归档意味着它不再是施工口径，所以那句话被换掉了。
//!
//! 与工具 / 模型 / 提示词**共用同一次 `traverse(TRAVERSE_AVAILABLE_TOOLS)` 广播**：
//! 容器在这一次广播里为每个交得出 VDFS 视图（`Plugin::get_vfs_provider` 返回 `Some`）
//! 的子插件**派生**挂载行——挂载名取实例表挂载名，插件自己不再注册；
//! 根行同样由容器（装配点）写入。写口因此仍只有 `Registry::put` 一处。

use super::{CapabilityRowKind, Registry};
use crate::symbio_core::VdfsProvider;
use std::sync::Arc;

/// 取 VDFS 根 provider（**服务 `<根>` 的那一个**）。
///
/// 无容器注册时为 `None`（等价于「系统里没有资源」）——调用方给空根降级
/// （`plugins/vdfs/host.rs::root_of`），不在这里替它造一个。
pub async fn capability_vdfs_root(visitor: &dyn Registry) -> Option<Arc<dyn VdfsProvider>> {
    visitor
        .list(CapabilityRowKind::Mount)
        .await
        .into_iter()
        .find(|row| row.active())
        .and_then(|row| row.vdfs().cloned())
}

/// 列出已注册的挂载点：`(目录名, 实现)`，按注册顺序（= 广播序）。
///
/// 根**不在**这张清单里：它不是「一个挂载点」，是服务整棵子树的那一个（见模块文档）。
pub async fn capability_mounts(visitor: &dyn Registry) -> Vec<(String, Arc<dyn VdfsProvider>)> {
    visitor
        .list(CapabilityRowKind::Mount)
        .await
        .into_iter()
        .filter(|row| !row.active())
        .filter_map(|row| {
            row.vdfs()
                .map(|provider| (row.name().to_string(), Arc::clone(provider)))
        })
        .collect()
}

/// 按目录名查挂载点（根不可按名查：它的行名是保留值，不对外）。
pub async fn capability_mount(visitor: &dyn Registry, name: &str) -> Option<Arc<dyn VdfsProvider>> {
    let row = visitor.get(name).await?;
    if row.active() {
        return None;
    }
    row.vdfs().cloned()
}
