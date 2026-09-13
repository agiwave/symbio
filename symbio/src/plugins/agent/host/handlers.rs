//! bundle 管理路由（agent/bundle/*）。
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
//! 资源（含 bundle 内部的 prompts / skills / mcps）的**访问**不再经本协议：
//! 一律走 VDFS（`.vdfs/agent/…`），由 `EntityVdfsAdapter` 直接调 trait 的
//! `*_container_item` 钩子；`entities/*` 协议已随 S11 下线。
use super::store::BundleStore;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload, WORKDIR,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// 路由分发（`path` 已剥去 `agent/` 前缀）。
pub async fn route(path: &str, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
    let workdir = ctx.get(WORKDIR);
    let store = BundleStore::new(workdir.as_deref());

    match path {
        // ── 插件自有管理路由 ──
        //
        // 资源访问一律经 VDFS（`.vdfs/agent/…`）；`entities/*` 协议已随 S11 下线，
        // 本插件不再托管它的任何分支（子实体的读写由 `EntityVdfsAdapter` 直接
        // 调 trait 钩子完成，不经协议）。
        "bundle/list" => list(&store),
        "bundle/get" => get(&store, &ctx).await,
        "bundle/upload" => upload(&store, &ctx).await,
        "bundle/export" => export(&store, &ctx).await,
        "bundle/delete" => delete(&store, &ctx).await,
        "bundle/preview" => preview(&store, &ctx).await,
        _ => Err(PluginError::NotFound(format!(
            "agent 未知路由 `{path}`（可用：bundle/list|get|upload|export|delete|preview）"
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
