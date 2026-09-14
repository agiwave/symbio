//! 实体机制接入（`kind = agent`）—— 供 VDFS 挂载点使用。
//!
//! ## 与 [`super::handlers`] 的分工
//!
//! - [`EntityProvider::list_items`]：本模块 override，直接枚举 [`BundleStore`]
//!   （bundle 不是 EntityStore 型，不落 `~/.symbio/plugins/<category>/`）；
//! - 写 / 删：bundle 由 [`BundleStore`] 自管目录与 manifest 校验（zip-slip
//!   防护 / 版本硬门槛），故不复用默认的 EntityStore 写盘路径，响应形状与
//!   实体机制保持一致（`EntitySummary` / `EntityUploadResponse`）。
//!
//! `entities/*` 调用协议已随 S11 下线：本模块的钩子**只**由
//! `EntityVdfsAdapter` 调用，外部访问一律走 `.vdfs/agent/…`。

use super::plugin::AgentPlugin;
use super::store::{classify_entity_path, BundleStore};
use crate::symbio_core::entities::{
    EntityExport, EntityProvider, EntitySummary, EntityUploadResponse,
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
    let mut counts = ContainerCounts {
        prompt: 0,
        skill: 0,
        mcp: 0,
    };
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
                // config_type = "bundle"：项级分发键，前端据此分发，必须保留；
                // 明细展示走 DetailForm info 绑定
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

    /// 删除 bundle（VDFS `vdfs/delete` → `entity_delete` 走这里）。
    ///
    /// bundle 不是 EntityStore 型，默认的「按分类删目录」不适用，故自管删除
    /// （原 `bundle/delete` 的落盘逻辑，随 S12 收敛到实体钩子里）。
    async fn delete_item(&self, ctx: &Arc<dyn InvokeRequest>, id: &str) -> Result<(), PluginError> {
        let store = Self::store_of(ctx);
        store.delete(id).map_err(PluginError::ValidationError)?;
        Ok(())
    }

    /// 整包导入：zip → [`BundleStore::import`]（id 取自包内 manifest，故忽略建议名）。
    ///
    /// 这是 bundle **唯一的创建方式**——bundle 是整目录能力包，没有「先建空壳
    /// 再填字段」的形态，因此挂载根只声明 `ext = zip` 一种新建类型。
    /// 已存在同名 bundle 时**替换**（`replace = true`），与导入语义一致。
    async fn import_zip(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        _name: &str,
        zip: &[u8],
    ) -> Result<EntityUploadResponse, PluginError> {
        let store = Self::store_of(ctx);
        let r = store
            .import(zip, true)
            .map_err(PluginError::ValidationError)?;
        Ok(EntityUploadResponse {
            kind: self.kind().to_string(),
            id: r.id,
            created: !r.replaced,
        })
    }

    /// 整包导出：bundle → zip（VDFS 节点动作 `export` 走这里）。
    ///
    /// bundle 目录由 [`BundleStore`] 自管（工作区级 / 全局级双层），默认的
    /// EntityStore 打包路径不适用，故直接委托 [`BundleStore::export`]——
    /// 与 [`Self::import_zip`] 互为逆向。
    async fn export_zip(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Result<EntityExport, PluginError> {
        let store = Self::store_of(ctx);
        let bytes = store.export(id).map_err(PluginError::ValidationError)?;
        Ok(EntityExport {
            id: id.to_string(),
            filename: format!("{id}.zip"),
            b64: crate::symbio_core::entities::encode_b64(&bytes),
        })
    }

    // ==================== 容器子实体（bundle 内部文件） ====================
    //
    // bundle 条目即容器：内部 prompts / skills / mcps 由 `EntityVdfsAdapter` 以
    // `<bundle id>/<子类别标签>/<相对路径>` 寻址后直调下面四个钩子（不经协议），
    // 实现复用 BundleStore 的沙箱化方法（路径白名单 classify_entity_path +
    // absolutize 双重闸门）。

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
