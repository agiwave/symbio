//! VFDS 插件 —— 虚拟文件系统的**访问层**（协议入口 + LLM 工具）
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

use super::{host, protocol as p, provider::ToolVdfs, tools};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CAPABILITY_VISITOR, PATH, PLUGIN_VFDS, TRAVERSE_AVAILABLE_TOOLS,
};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// VFDS 插件：把**容器注册的 VDFS 根**以一组 `vdfs/*` 操作暴露给前端与 LLM。
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
        let path = ctx.get(PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path).to_string();

        // 调用方寻址为 `vdfs/<op>`，composite 已剥掉 `vdfs/` 前缀 → 此处补回，
        // 使协议路径常量（VFDS_OPS）与实现共享同一定义。
        let op = if path.starts_with("vdfs/") {
            path.clone()
        } else {
            format!("vdfs/{path}")
        };
        if !p::VFDS_OPS.contains(&op.as_str()) {
            return Err(PluginError::NotFound(format!("VFDS: 未知路径 '{path}'")));
        }

        // 取容器注册的 VDFS 根：本插件只转发，不认识任何挂载点
        let parent = self.get_parent().await;
        let root = host::resolve_root(parent.as_ref(), &ctx).await;

        // 前端给的是**全路径**（如 `/local/README.md`），因此不做挂载点前缀；
        // 但运行时状态（workdir）仍要透传，否则 local provider 解析不了相对路径。
        let params = host::call_params(&ctx);

        host::dispatch_with(&root, &op, &ctx, params)
            .await
            .unwrap_or_else(|| {
                Err(PluginError::InternalError(format!(
                    "VFDS: 协议路径未分发 {op}"
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

crate::submit_object_creator!(PLUGIN_VFDS, VdfsPlugin::build, dyn Plugin);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::SimpleRequest;

    fn build_plugin() -> Arc<dyn Plugin> {
        VdfsPlugin::build(Arc::new(SimpleRequest::new(None, None)))
    }

    fn ctx_with_path(path: &str) -> Arc<dyn InvokeRequest> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, path.to_string());
        ctx
    }

    #[tokio::test]
    async fn route_unknown_path_is_not_found() {
        let err = build_plugin()
            .route(ctx_with_path("bogus"))
            .await
            .unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)));
    }

    /// 无父插件（未挂载）时 `vdfs/providers` 返回空清单而非报错——
    /// 挂载点集合由 provider 注册决定，宿主自身不持有任何资源。
    #[tokio::test]
    async fn providers_without_parent_is_empty() {
        let resp = build_plugin()
            .route(ctx_with_path("providers"))
            .await
            .unwrap();
        let data = resp
            .get::<p::VdfsProvidersResponse>()
            .expect("应为 providers 响应");
        assert!(data.providers.is_empty());
    }

    /// 未知挂载点 → NotFound（提示现有挂载点）
    #[tokio::test]
    async fn list_unknown_mount_errors() {
        let ctx = ctx_with_path("list");
        ctx.set_payload(serde_json::json!({ "path": "/nope" }))
            .unwrap();
        let err = build_plugin().route(ctx).await.unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)));
    }

    /// 虚拟根 `/` 恒可列出（即使无挂载点）
    #[tokio::test]
    async fn list_root_always_ok() {
        let ctx = ctx_with_path("list");
        ctx.set_payload(serde_json::json!({ "path": "/" })).unwrap();
        let resp = build_plugin().route(ctx).await.unwrap();
        let data = resp.get::<p::VdfsListResponse>().unwrap();
        assert_eq!(data.path, "/");
        assert!(data.items.is_empty());
    }

    /// 工具集：每个 VDFS 操作恰好一个工具
    #[test]
    fn tools_cover_all_ops() {
        let visitor: Arc<dyn crate::symbio_core::CapabilityVisitor> =
            Arc::new(crate::symbio_core::DefaultToolVisitor::new());
        let names: Vec<String> = tools::vdfs_tools(Arc::new(ToolVdfs::new(visitor)))
            .iter()
            .map(|t| t.name())
            .collect();
        assert_eq!(
            names,
            vec![
                "vdfs_list",
                "vdfs_tree",
                "vdfs_stat",
                "vdfs_read",
                "vdfs_edit",
                "vdfs_search",
                "vdfs_write",
                "vdfs_delete",
                "vdfs_mkdir",
                "vdfs_move",
            ]
        );
    }
}
