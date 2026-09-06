//! 统一实体协议（`entities/*`）接入 —— kind = `agent`。
//!
//! ## 与 [`super::handlers`] 的分工
//!
//! - `entities/list`：走 [`crate::symbio_core::entities::dispatch`] 公共流程，
//!   本模块 override [`EntityProvider::list_items`] 直接枚举 [`BundleStore`]；
//! - `entities/get` / `upload` / `delete`：dispatch 默认实现走 EntityStore
//!   （`category()` 语义），而 bundle 由 [`BundleStore`] 自管目录与 manifest
//!   校验（zip-slip 防护 / 版本硬门槛），故由 handlers 直接拦截实现，
//!   响应形状与统一协议保持一致（`EntitySummary` / `EntityUploadResponse`）。

use super::plugin::AgentPlugin;
use super::store::{classify_entity_path, BundleStore};
use crate::symbio_core::entities::{
    EntityProvider, EntitySummary, EntityUploadResponse,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt, PluginError, WORKDIR};
use async_trait::async_trait;
use std::sync::Arc;

impl AgentPlugin {
    /// 依请求上下文构造 BundleStore（每次请求独立，与 route 入口一致）
    fn store_of(ctx: &Arc<dyn InvokeRequest>) -> BundleStore {
        BundleStore::new(ctx.get(WORKDIR).as_deref())
    }
}

/// bundle 内部实体计数（提示词/技能/MCP；列表与概览定义共用）
struct ContainerCounts {
    prompt: usize,
    skill: usize,
    mcp: usize,
}

fn container_counts(store: &BundleStore, bundle_id: &str) -> ContainerCounts {
    let mut counts = ContainerCounts { prompt: 0, skill: 0, mcp: 0 };
    if let Ok(entries) = store.list_entities(bundle_id) {
        for e in entries {
            match e.kind.as_str() {
                "prompt" => counts.prompt += 1,
                "skill" => counts.skill += 1,
                "mcp" => counts.mcp += 1,
                _ => {}
            }
        }
    }
    counts
}

#[async_trait]
impl EntityProvider for AgentPlugin {
    fn kind(&self) -> &'static str {
        crate::symbio_core::entities::ENTITY_AGENT
    }

    fn category(&self) -> Option<&'static str> {
        // bundle 由 BundleStore 自管目录（工作区级 + 全局级双层），不走 EntityStore
        None
    }

    /// 详情页定义：bundle 只读概览（info 绑定，见 `super::detail`）——
    /// 概览字段 + 内部实体计数 + open-container/delete 机制动作。
    /// 新建态（id 为空）返回 None：agent 新建保留 zip 上传流程。
    async fn detail_definition(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Option<crate::symbio_core::schemas::entities::DetailDefinition> {
        if id.is_empty() {
            return None;
        }
        Some(super::detail::agent_detail_definition())
    }

    /// 列出全部 bundle：工作区级覆盖同名全局级，逐条从 manifest 提取摘要。
    async fn list_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<EntitySummary>, PluginError> {
        let workdir = ctx.get(crate::symbio_core::WORKDIR);
        let store = BundleStore::new(workdir.as_deref());
        Ok(store
            .list()
            .into_iter()
            .map(|r| {
                let mut it = EntitySummary::new(
                    crate::symbio_core::entities::ENTITY_AGENT,
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
                // 类型特有扩展：版本 / 规格 / 来源层级 / 安装目录 / 内部实体计数
                // （前端 DetailForm info 绑定按需展示）
                // config_type = "bundle"：项级分发键（历史上指向 agent:bundle
                // editor，现已机制化为 DetailForm info 绑定，保留兼容）
                let counts = container_counts(&store, &r.manifest.id);
                it.extra = serde_json::json!({
                    "config_type": "bundle",
                    "version": r.manifest.version,
                    "spec": r.manifest.spec,
                    "requires_spec": r.manifest.requires.spec,
                    "scope": r.source.as_str(),
                    "dir": r.dir.to_string_lossy(),
                    "count_prompt": counts.prompt,
                    "count_skill": counts.skill,
                    "count_mcp": counts.mcp,
                });
                it
            })
            .collect())
    }

    // ==================== 容器子实体（统一协议 container 语义） ====================
    //
    // bundle 条目即容器：内部 prompts / skills / mcps 经同一套 entities/* 协议
    // 访问（payload.container = bundle id），复用 BundleStore 的沙箱化方法
    // （路径白名单 classify_entity_path + absolutize 双重闸门）。

    async fn list_container_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        sub_kind: Option<&str>,
        container: &str,
        _parent: Option<&str>,
    ) -> Result<Vec<EntitySummary>, PluginError> {
        let store = Self::store_of(ctx);
        let entries = store
            .list_entities(container)
            .map_err(PluginError::ValidationError)?;
        Ok(entries
            .into_iter()
            .filter(|e| sub_kind.is_none_or(|k| e.kind == k))
            .map(|e| {
                let mut it = EntitySummary::new(&e.kind, e.path.clone(), e.name.clone());
                it.status = "active".to_string();
                it.extra = serde_json::json!({
                    "container": container,
                    "priority": e.priority,
                    "size": e.size,
                });
                it
            })
            .collect())
    }

    async fn get_container_item(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        container: &str,
    ) -> Result<EntitySummary, PluginError> {
        let store = Self::store_of(ctx);
        let (kind, name) = classify_entity_path(id).map_err(PluginError::ValidationError)?;
        let content = store
            .read_entity(container, id)
            .map_err(PluginError::ValidationError)?;
        let mut it = EntitySummary::new(kind, id, name);
        it.status = "active".to_string();
        it.extra = serde_json::json!({
            "container": container,
            "content": content,
        });
        Ok(it)
    }

    async fn put_container_item(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        content: &str,
        container: &str,
    ) -> Result<EntityUploadResponse, PluginError> {
        let store = Self::store_of(ctx);
        let (kind, _) = classify_entity_path(id).map_err(PluginError::ValidationError)?;
        let existed = store
            .list_entities(container)
            .map_err(PluginError::ValidationError)?
            .iter()
            .any(|e| e.path == id);
        store
            .write_entity(container, id, content)
            .map_err(PluginError::ValidationError)?;
        Ok(EntityUploadResponse {
            kind: kind.to_string(),
            id: id.to_string(),
            created: !existed,
        })
    }

    async fn delete_container_item(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        container: &str,
    ) -> Result<EntityUploadResponse, PluginError> {
        let store = Self::store_of(ctx);
        let (kind, _) = classify_entity_path(id).map_err(PluginError::ValidationError)?;
        store
            .delete_entity(container, id)
            .map_err(PluginError::ValidationError)?;
        Ok(EntityUploadResponse {
            kind: kind.to_string(),
            id: id.to_string(),
            created: false,
        })
    }
}
