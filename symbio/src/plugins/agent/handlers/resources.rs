//! Agent 统一资源协议（resources/*）：ResourceProvider trait 实现。
//!
//! 公共流程（zip 上传 / 幂等删除 / 列表包装）由 `dispatch` 承载，
//! 这里只实现 Agent 的差异化钩子：列表来自 AgentManager（Registry + 索引缓存），
//! 上传/删除后失效索引缓存。

use crate::plugins::agent::manager::AgentRegistry;
use crate::plugins::agent::plugin::AgentPlugin;
use crate::symbio_core::resources::{ResourceProvider, ResourceSummary, RESOURCE_AGENT};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginError};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
impl ResourceProvider for AgentPlugin {
    fn kind(&self) -> &'static str {
        RESOURCE_AGENT
    }

    fn category(&self) -> Option<&'static str> {
        Some(crate::symbio_core::providers::categories::AGENT)
    }

    /// Agent 列表来自 AgentManager（Registry 初始化 + 索引缓存），不走 EntityStore 枚举
    async fn list_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<ResourceSummary>, PluginError> {
        let workdir = ctx.get(crate::symbio_core::WORKDIR);
        {
            let agent_config = self.config.read().await;
            let _ =
                AgentRegistry::ensure_initialized(&self.manager, workdir.as_deref(), &agent_config)
                    .await;
        }
        let agents = self.manager.list_agents(workdir.as_deref()).await;

        Ok(agents
            .into_iter()
            .map(|a| {
                let mut it = ResourceSummary::new(RESOURCE_AGENT, &a.id, a.name.clone());
                it.description = Some(a.description.clone());
                it.status = "active".to_string();
                it
            })
            .collect::<Vec<_>>())
    }

    /// 上传后失效 AgentIndex 列表缓存（TTL 10 分钟），确保新资源立即可见
    async fn on_uploaded(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<(), PluginError> {
        self.manager.invalidate_cache_for_workdir(None).await;
        Ok(())
    }

    /// 删除后同样失效缓存
    async fn on_deleted(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<(), PluginError> {
        self.manager.invalidate_cache_for_workdir(None).await;
        Ok(())
    }
}
