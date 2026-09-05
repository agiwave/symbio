//! 统一资源协议（`resources/*`）接入 —— kind = `agent`。
//!
//! ## 与 [`super::handlers`] 的分工
//!
//! - `resources/list`：走 [`crate::symbio_core::resources::dispatch`] 公共流程，
//!   本模块 override [`ResourceProvider::list_items`] 直接枚举 [`BundleStore`]；
//! - `resources/get` / `upload` / `delete`：dispatch 默认实现走 EntityStore
//!   （`category()` 语义），而 bundle 由 [`BundleStore`] 自管目录与 manifest
//!   校验（zip-slip 防护 / 版本硬门槛），故由 handlers 直接拦截实现，
//!   响应形状与统一协议保持一致（`ResourceSummary` / `ResourceUploadResponse`）。

use super::plugin::AgentPlugin;
use super::store::{BundleScope, BundleStore};
use crate::symbio_core::resources::{ResourceProvider, ResourceSummary};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginError};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
impl ResourceProvider for AgentPlugin {
    fn kind(&self) -> &'static str {
        crate::symbio_core::resources::RESOURCE_AGENT
    }

    fn category(&self) -> Option<&'static str> {
        // bundle 由 BundleStore 自管目录（工作区级 + 全局级双层），不走 EntityStore
        None
    }

    /// 列出全部 bundle：工作区级覆盖同名全局级，逐条从 manifest 提取摘要。
    async fn list_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<ResourceSummary>, PluginError> {
        let workdir = ctx.get(crate::symbio_core::WORKDIR);
        let store = BundleStore::new(workdir.as_deref());
        Ok(store
            .list()
            .into_iter()
            .map(|r| {
                let mut it = ResourceSummary::new(
                    crate::symbio_core::resources::RESOURCE_AGENT,
                    r.manifest.id.clone(),
                    if r.manifest.name.is_empty() {
                        r.manifest.id.clone()
                    } else {
                        r.manifest.name.clone()
                    },
                );
                it.status = "active".to_string();
                if !r.manifest.description.is_empty() {
                    it.description = Some(r.manifest.description.clone());
                    it.summary = Some(r.manifest.description.clone());
                }
                // 类型特有扩展：版本 / 规格 / provider 数 / 来源层级（前端按需展示）
                it.extra = serde_json::json!({
                    "version": r.manifest.version,
                    "spec": r.manifest.spec,
                    "requires_spec": r.manifest.requires.spec,
                    "scope": match r.source {
                        BundleScope::Workspace => "workspace",
                        BundleScope::Global => "global",
                    },
                });
                it
            })
            .collect())
    }
}
