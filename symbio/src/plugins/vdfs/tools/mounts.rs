//! `vdfs_mounts` —— 列出虚拟文件系统的全部挂载点（资源类别）。
//!
//! 封装 provider 的 `mounts()` 直接给出 `(挂载名, provider)` 清单；本工具把它们
//! 装成便于 LLM 消费的挂载点视图（名称 / 标签 / 访问位 / 根路径）。

use super::{tool, ToolVdfs};
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeResponse, PluginPayload,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// 列出挂载点工具（框架原生 `Capability`）
pub struct MountsTool {
    provider: Arc<ToolVdfs>,
}

impl MountsTool {
    pub fn new(provider: Arc<ToolVdfs>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Capability for MountsTool {
    fn meta(&self) -> CapabilityMeta {
        tool(
            "vdfs_mounts",
            "列出虚拟文件系统的全部挂载点（资源类别）及其访问位（r=读 w=写 l=列表 t=遍历）。本工具不接收路径，用于确认当前系统可访问哪些类别；其余工具的裸路径解析到本地工作目录，'.vdfs/<挂载名>/...' 可访问其它类别。",
            json!({ "type": "object", "properties": {} }),
            vec!["{}"],
            None,
        )
    }

    async fn execute(&self, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        use super::super::protocol::{VdfsMountInfo, VdfsProvidersResponse};
        let providers = self
            .provider
            .mounts()
            .await
            .into_iter()
            .enumerate()
            .map(|(i, (mount, provider))| VdfsMountInfo {
                root: format!("/{mount}"),
                label: provider.label().unwrap_or(&mount).to_string(),
                description: provider.description().map(str::to_string),
                order: i as i32,
                access: provider.root_access(),
                status: provider.root_status().to_string(),
                icon: provider.icon().map(str::to_string),
                new_types: provider.root_new_types(),
                nav_visible: provider.nav_visible(),
                mount,
                attributes: Default::default(),
            })
            .collect();
        Ok(PluginPayload::new(&VdfsProvidersResponse { providers }))
    }
}
