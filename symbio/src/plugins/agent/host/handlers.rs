//! bundle 管理路由（agent/bundle/* 与统一资源协议 agent/resources/*）。
//!
//! ## bundle/* 语义
//!
//! | 路由 | 语义 |
//! |---|---|
//! | `bundle/list` | 全量 bundle 列表（工作区级覆盖全局级） |
//! | `bundle/get` | 单 bundle 详情（manifest 完整内容） |
//! | `bundle/upload` | zip 导入（`zip_b64` base64；`replace` 控制覆盖） |
//! | `bundle/export` | 打包下载（返回 `zip_b64` base64） |
//! | `bundle/delete` | 删除已安装 bundle |
//! | `bundle/preview` | manifest + provider 文件清单 |
//!
//! ## resources/* 语义（统一资源协议，前端资源管理页使用）
//!
//! 顶层语义：`list` 走 [`crate::symbio_core::resources::dispatch`] 公共流程；
//! `get` / `upload` / `delete` 由 BundleStore 拦截实现（自带 manifest 校验与
//! zip-slip 防护），响应形状与统一协议一致（见 host/resources.rs 模块文档）。
//!
//! **容器语义**（请求携带 `container` = bundle id）：bundle 内部的
//! prompts / skills / mcps 走同一套 resources/* 协议（trait 的
//! `*_container_item` 钩子，见 host/resources.rs），响应形状与顶层一致。
//! `list` 的容器分支直接在 dispatch 内完成；`get` / `upload` / `delete`
//! 因顶层语义由本模块拦截，容器语义时委托回 dispatch。

use super::plugin::AgentPlugin;
use super::store::BundleStore;
use crate::symbio_core::resources::{
    dispatch, ResourceDeleteRequest, ResourceGetRequest, ResourceSummary, ResourceUploadRequest,
    ResourceUploadResponse,
};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload, WORKDIR,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// 路由分发（`path` 已剥去 `agent/` 前缀）。
pub async fn route(
    plugin: &AgentPlugin,
    path: &str,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let workdir = ctx.get(WORKDIR);
    let store = BundleStore::new(workdir.as_deref());

    match path {
        // ── 统一资源协议（前端资源管理页；list 容器分支在 dispatch 内）──
        "resources/list" => {
            // 公共流程：list_items（BundleStore 枚举）+ provider 回填 + 能力开关；
            // path 命中 RESOURCES_LIST，dispatch 必返回 Some
            match dispatch(plugin, path, &ctx).await {
                Some(resp) => resp,
                None => Err(PluginError::InternalError(
                    "resources/list dispatch 失败".into(),
                )),
            }
        }
        "resources/get" => {
            let req: ResourceGetRequest = ctx.payload()?;
            if req.container.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                // 容器语义：委托 dispatch 容器分支（trait get_container_item）
                match dispatch(plugin, path, &ctx).await {
                    Some(resp) => resp,
                    None => Err(PluginError::InternalError(
                        "resources/get dispatch 失败".into(),
                    )),
                }
            } else {
                resources_get(&store, &ctx).await
            }
        }
        "resources/upload" => {
            let req: ResourceUploadRequest = ctx.payload()?;
            if req.container.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                match dispatch(plugin, path, &ctx).await {
                    Some(resp) => resp,
                    None => Err(PluginError::InternalError(
                        "resources/upload dispatch 失败".into(),
                    )),
                }
            } else {
                resources_upload(&store, &ctx).await
            }
        }
        "resources/delete" => {
            let req: ResourceDeleteRequest = ctx.payload()?;
            if req.container.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                match dispatch(plugin, path, &ctx).await {
                    Some(resp) => resp,
                    None => Err(PluginError::InternalError(
                        "resources/delete dispatch 失败".into(),
                    )),
                }
            } else {
                resources_delete(&store, &ctx).await
            }
        }

        // ── 插件自有管理路由 ──
        "bundle/list" => list(&store),
        "bundle/get" => get(&store, &ctx).await,
        "bundle/upload" => upload(&store, &ctx).await,
        "bundle/export" => export(&store, &ctx).await,
        "bundle/delete" => delete(&store, &ctx).await,
        "bundle/preview" => preview(&store, &ctx).await,
        _ => Err(PluginError::NotFound(format!(
            "agent 未知路由 `{path}`（可用：bundle/list|get|upload|export|delete|preview、\
             resources/list|get|upload|delete）"
        ))),
    }
}

fn summary_of(store: &BundleStore) -> impl Iterator<Item = Value> + '_ {
    store.list().into_iter().map(|r| {
        json!({
            "id": r.manifest.id,
            "name": r.manifest.name,
            "version": r.manifest.version,
            "spec": r.manifest.spec,
            "requires_spec": r.manifest.requires.spec,
            "description": r.manifest.description,
            "scope": r.source.as_str(),
            "dir": r.dir.display().to_string(),
        })
    })
}

fn list(store: &BundleStore) -> InvokeResponse<PluginPayload> {
    Ok(PluginPayload::new(&json!({
        "bundles": summary_of(store).collect::<Vec<_>>(),
    })))
}

async fn get(store: &BundleStore, ctx: &Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
    #[derive(Deserialize, Clone)]
    struct GetRequest {
        id: String,
    }
    let req: GetRequest = ctx.payload()?;
    let record = store
        .get(&req.id)
        .ok_or_else(|| PluginError::NotFound(format!("bundle `{}` 不存在", req.id)))?;
    let manifest_value = serde_json::to_value(record.manifest.as_ref())
        .map_err(|e| PluginError::InternalError(format!("manifest 序列化失败: {e}")))?;
    Ok(PluginPayload::new(&json!({
        "manifest": manifest_value,
        "scope": record.source.as_str(),
        "dir": record.dir.display().to_string(),
    })))
}

async fn upload(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    #[derive(Deserialize, Clone)]
    struct UploadRequest {
        /// zip 字节（base64）
        zip_b64: String,
        /// 已存在时是否覆盖（默认 false）
        #[serde(default)]
        replace: bool,
    }
    let req: UploadRequest = ctx.payload()?;
    let zip_bytes = base64::engine::general_purpose::STANDARD
        .decode(req.zip_b64.trim())
        .map_err(|e| PluginError::ValidationError(format!("zip_b64 解码失败: {e}")))?;
    let result = store
        .import(&zip_bytes, req.replace)
        .map_err(PluginError::ValidationError)?;
    Ok(PluginPayload::new(&serde_json::to_value(&result).map_err(
        |e| PluginError::InternalError(format!("结果序列化失败: {e}")),
    )?))
}

async fn export(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    #[derive(Deserialize, Clone)]
    struct ExportRequest {
        id: String,
    }
    let req: ExportRequest = ctx.payload()?;
    let zip_bytes = store
        .export(&req.id)
        .map_err(PluginError::ValidationError)?;
    let zip_b64 = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
    Ok(PluginPayload::new(&json!({
        "id": req.id,
        "zip_b64": zip_b64,
    })))
}

async fn delete(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    #[derive(Deserialize, Clone)]
    struct DeleteRequest {
        id: String,
    }
    let req: DeleteRequest = ctx.payload()?;
    let dir = store
        .delete(&req.id)
        .map_err(PluginError::ValidationError)?;
    Ok(PluginPayload::new(&json!({
        "deleted": true,
        "id": req.id,
        "dir": dir,
    })))
}

async fn preview(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    #[derive(Deserialize, Clone)]
    struct PreviewRequest {
        id: String,
    }
    let req: PreviewRequest = ctx.payload()?;
    let record = store
        .get(&req.id)
        .ok_or_else(|| PluginError::NotFound(format!("bundle `{}` 不存在", req.id)))?;

    // 约定目录内容清单（prompts/ skills/ mcps/，相对 bundle 目录）
    let mut files: Vec<String> = Vec::new();
    for dir in ["prompts", "skills", "mcps"] {
        collect_files(&record.dir.join(dir), dir, &mut files);
    }
    files.sort();

    let manifest_value = serde_json::to_value(record.manifest.as_ref())
        .map_err(|e| PluginError::InternalError(format!("manifest 序列化失败: {e}")))?;
    Ok(PluginPayload::new(&json!({
        "manifest": manifest_value,
        "files": files,
    })))
}

fn collect_files(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = format!("{prefix}/{name}");
        if path.is_dir() {
            collect_files(&path, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

// ==================== 统一资源协议实现（get / upload / delete） ====================

/// `resources/get`：单 bundle 摘要（ResourceSummary 形状，含 manifest 扩展字段）。
async fn resources_get(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceGetRequest = ctx.payload()?;
    let record = store
        .get(&req.id)
        .ok_or_else(|| PluginError::NotFound(format!("bundle `{}` 不存在", req.id)))?;
    let mut it = ResourceSummary::new(
        crate::symbio_core::resources::RESOURCE_AGENT,
        record.manifest.id.clone(),
        if record.manifest.name.is_empty() {
            record.manifest.id.clone()
        } else {
            record.manifest.name.clone()
        },
    );
    it.status = "active".to_string();
    if !record.manifest.description.is_empty() {
        it.description = Some(record.manifest.description.clone());
        it.summary = Some(record.manifest.description.clone());
    }
    it.extra = serde_json::json!({
        "config_type": "bundle",
        "version": record.manifest.version,
        "spec": record.manifest.spec,
        "requires_spec": record.manifest.requires.spec,
        "scope": record.source.as_str(),
        "dir": record.dir.display().to_string(),
    });
    Ok(PluginPayload::new(&it))
}

/// `resources/upload`：zip 导入（走 BundleStore，manifest 校验 + 版本硬门槛）。
async fn resources_upload(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceUploadRequest = ctx.payload()?;
    let zip_b64 = req.zip_b64.as_deref().ok_or_else(|| {
        PluginError::ValidationError("bundle 导入需要 zip_b64（zip 打包）".into())
    })?;
    let zip_bytes = base64::engine::general_purpose::STANDARD
        .decode(zip_b64.trim())
        .map_err(|e| PluginError::ValidationError(format!("zip_b64 解码失败: {e}")))?;
    let result = store
        .import(&zip_bytes, req.replace)
        .map_err(PluginError::ValidationError)?;
    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: crate::symbio_core::resources::RESOURCE_AGENT.to_string(),
        id: result.id,
        created: !result.replaced,
    }))
}

/// `resources/delete`：幂等删除（不存在时报 NotFound，与其它资源类型语义一致）。
async fn resources_delete(
    store: &BundleStore,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceDeleteRequest = ctx.payload()?;
    if store.get(&req.id).is_none() {
        return Err(PluginError::NotFound(format!("bundle `{}` 不存在", req.id)));
    }
    store
        .delete(&req.id)
        .map_err(PluginError::ValidationError)?;
    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: crate::symbio_core::resources::RESOURCE_AGENT.to_string(),
        id: req.id,
        created: false,
    }))
}
