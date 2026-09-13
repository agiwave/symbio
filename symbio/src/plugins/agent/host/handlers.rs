//! bundle 管理路由（agent/bundle/*）。
//!
//! ## bundle/* 语义
//!
//! | 路由 | 语义 |
//! |---|---|
//! | `bundle/export` | 打包下载（返回 `zip_b64` base64） |
//!
//! 其余管理动作**都已由 VDFS 承担**，故本协议不再重复提供（S12 清理）：
//!
//! | 原路由 | 现在走 |
//! |---|---|
//! | `bundle/list` / `bundle/get` / `bundle/preview` | `.vdfs/agent` 的 `vdfs/list` / `vdfs/read`（含内部 prompts / skills / mcps） |
//! | `bundle/upload` | 「新建类型 `zip`」——一次 `vdfs/write`（二进制）→ `EntityProvider::import_zip` |
//! | `bundle/delete` | `vdfs/delete` → `entity_delete` |
//!
//! 导出（打包下载）是**唯一还没有 VDFS 等价物**的动作：等 `vdfs/action` 承接
//! 「导出」动词后，本文件即可整体下线。
use super::store::BundleStore;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload, WORKDIR,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// 路由分发（`path` 已剥去 `agent/` 前缀）。
pub async fn route(path: &str, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
    let workdir = ctx.get(WORKDIR);
    let store = BundleStore::new(workdir.as_deref());

    match path {
        // ── 插件自有管理路由 ──
        //
        // 资源的访问 / 新建 / 删除一律经 VDFS（`.vdfs/agent/…`）：`entities/*`
        // 协议已随 S11 下线，bundle 的 list / get / upload / delete / preview
        // 又随 S12 被 VDFS 取代，这里只剩尚无等价物的导出。
        "bundle/export" => export(&store, &ctx).await,
        _ => Err(PluginError::NotFound(format!(
            "agent 未知路由 `{path}`（可用：bundle/export）"
        ))),
    }
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
