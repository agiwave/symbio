//! VDFS 插件 —— 虚拟文件系统的**访问层**（协议入口 + LLM 工具）
//!
//! ## 职责
//!
//! 1. **协议入口**：`vdfs/*` → 取容器注册的 VDFS 根 → `host::dispatch_with`（前端链路）
//! 2. **工具供给**：`traverse` 中构造封装 provider（`provider::ToolVdfs`，持有本次
//!    广播的能力管理器）并注册 `vdfs_*` 工具（LLM 链路）
//!
//! 两条链路消费**同一批**注册的挂载 provider，因此不存在「前端支持而 LLM 不支持」
//! 的资源；差异只在呈现——前端拿域响应原样渲染，工具在 `execute` 内封装。
//!
//! 变更投递不经本插件：provider 的回调由访问层接到事件总线
//! （`host::event_bus_sink`），少一跳、少一个长驻任务。
//!
//! 本插件**不含任何资源语义**，也**不持有拓扑**：不认识会话 / 模型 / 设置，
//! 不认识挂载点，只认识「根 provider + 全路径」——虚拟根归组合容器所有。
//!
//! ## 但**根叫什么**归本插件
//!
//! 「拓扑归容器、命名归本插件」是一件事的两半：
//!
//! - **根之下有什么** = 容器的注册（`register_vdfs_root` / `register_vdfs_provider`）；
//! - **根挂在哪个地址上** = 本插件的挂载规则（[`VDFS_ADDR_ROOT`]），经
//!   [`AddrRootDecl`] **静态声明**——随二进制生效，早于任何插件实例构造，
//!   容器转发请求时据此改写子上下文的当前父地址（`symbio_core::vdfs::address`）。
//!
//! 因此「根改叫什么」只需改 [`VDFS_ADDR_ROOT`] 一行：其它插件的绝对地址一律
//! 从**上下文里的当前父地址 + 相对地址**拼出（容器在转发点改写），前端用
//! `vdfs/root` 拿到的**地址数据**，两边都不持有名字。

use super::{fs::VDFS_ADDR_ROOT, host, protocol as p, provider::ToolVdfs, tools};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CAPABILITY_VISITOR, PATH, PLUGIN_VDFS, TRAVERSE_AVAILABLE_TOOLS,
};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

// 根名的静态声明：字面量只在本插件内（fs::VDFS_ADDR_ROOT），编译期随二进制
// 生效——容器的转发点在任何实例构造之前就要用它。
crate::symbio_core::inventory::submit! {
    crate::symbio_core::vdfs::AddrRootDecl(VDFS_ADDR_ROOT)
}

/// VDFS 插件：把**容器注册的 VDFS 根**以一组 `vdfs/*` 操作暴露给前端与 LLM。
///
/// 它不持有挂载点表——根之下有什么，由组合容器决定（见 `plugins/composite/vdfs.rs`）。
pub struct VdfsPlugin {
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl VdfsPlugin {
    /// 静态工厂（`submit_object_creator!` 使用）
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let parent = ctx.parent();
        Arc::new(Self {
            parent: Arc::new(RwLock::new(parent)),
        })
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("vdfs", "虚拟文件系统")
            .with_description("统一资源与数据访问层（Virtual Dynamic File System）")
            .with_version("0.1.0")
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }
}

#[async_trait::async_trait]
impl Plugin for VdfsPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        // 调用方可能给协议全名（`vdfs/list`），也可能给短名（`list`——容器已剥掉
        // `vdfs/` 前缀）。两种都归一成协议全名，使 `VDFS_OPS` 与实现共享同一定义。
        let raw = ctx.get(PATH).unwrap_or_default();
        let op = format!(
            "vdfs/{}",
            raw.trim_start_matches('/').trim_start_matches("vdfs/")
        );
        if !p::VDFS_OPS.contains(&op.as_str()) {
            return Err(PluginError::NotFound(format!("VDFS: 未知路径 '{raw}'")));
        }

        // 取容器注册的统一文件系统：本插件只转发，不认识任何资源类别
        let parent = self.get_parent().await;
        let fs = host::resolve_fs(parent.as_ref(), &ctx).await;

        // 前端给的是**展示地址**（如 `.vdfsv2/session/x` 或 `README.md`），门面按前缀
        // 分流；运行时状态（workdir）仍要透传，物理层据此解析相对地址。
        let params = host::call_params(&ctx);

        host::dispatch_with(&fs, &op, &ctx, params)
            .await
            .unwrap_or_else(|| {
                Err(PluginError::InternalError(format!(
                    "VDFS: 协议路径未分发 {op}"
                )))
            })
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(format!("未知遍历路径: {sub_path}")));
        }

        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            // 封装 provider 持有本次广播的能力管理器；工具注册进同一个 visitor，
            // 执行时其中已注册好全部挂载 provider（容器同时注册的组合根供前端链路用）。
            let provider = Arc::new(ToolVdfs::new(visitor.clone()));
            for tool in tools::vdfs_tools(provider) {
                visitor.register(tool).await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_VDFS, VdfsPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
