//! 统一资源门面（协议 re-export + 通用工具）
//!
//! - 协议定义见 [`schemas::resources`]（共享给各插件 route 复用）
//! - 本模块提供 zip 上传的通用解压 / 实体目录写盘工具，让 mcp / skill / agent
//!   三类插件共享同一套"zip → `~/.symbio/plugins/<category>/<id>/`"机制，避免重复实现。
//! - [`ResourceProvider`] trait + [`dispatch`] 把 `resources/*` 五个操作的公共流程
//!   （列表包装、zip/manifest 上传、幂等删除、状态事件推送）收敛到核心层，
//!   各插件只实现差异化钩子（summarize / validate_manifest / on_uploaded /
//!   on_deleted / test_status）。

pub use crate::symbio_core::schemas::resources::*;

use crate::symbio_core::providers::{EntityStore, EntityStoreError, StorageService};
use crate::symbio_core::{
    create_object, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};
use async_trait::async_trait;
use base64::Engine;
use std::io::{Cursor, Read};
use std::sync::Arc;

/// 解析当前请求的存储服务（`~/.symbio/plugins/` 基座）
///
/// 各插件 resource handler 共享的 `es()` 助手的统一替代。
pub fn storage_service(
    ctx: &Arc<dyn InvokeRequest>,
) -> Result<Arc<dyn StorageService>, PluginError> {
    create_object::<dyn StorageService>("storage_service", ctx.clone())
        .ok_or_else(|| PluginError::InternalError("storage_service 不可用".to_string()))
}

// ==================== ResourceProvider trait ====================

/// 资源提供方 trait —— 各插件实现差异化钩子，公共流程由 [`dispatch`] 承载。
///
/// ## 默认实现与重写
///
/// - 默认 `list_items` 走 `EntityStore` 枚举 + [`Self::summarize`]（适合
///   mcp / skill 等纯目录资源）；model / session / agent 等有独立数据源的
///   重写 `list_items` 接管
/// - 默认 `upload` / `delete` 由 [`dispatch`] 基于 `category` + `manifest_file`
///   完成（zip 解压或 manifest 写盘、幂等删除）；无实体目录的资源
///   （`category() == None`，如 session）返回 `NotImplemented`，为将来
///   的导入/导出留协议槽位
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    /// 资源类型常量（RESOURCE_MODEL / RESOURCE_MCP / ...）
    fn kind(&self) -> &'static str;

    /// 提供方（插件）显示名，用于前端资源路径 `[provider]/[id].[kind]` 展示。
    /// 默认与 kind 相同；未来插件显示名与 kind 分叉时重写本方法即可。
    fn provider_name(&self) -> &str {
        self.kind()
    }

    /// EntityStore 分类；None = 非实体目录存储（session 走 SessionStore）
    fn category(&self) -> Option<&'static str> {
        None
    }

    /// manifest 文件名（`Some(category)` 时用于默认 get / manifest 写盘）
    fn manifest_file(&self) -> Option<&'static str> {
        None
    }

    /// 列出全部资源摘要。默认：EntityStore 枚举 + [`Self::summarize`]。
    async fn list_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<ResourceSummary>, PluginError> {
        let (Some(category), Some(manifest)) = (self.category(), self.manifest_file()) else {
            return Err(PluginError::NotImplemented);
        };
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        let ids = es
            .list_entities(category)
            .await
            .map_err(|e| PluginError::InternalError(format!("列出资源失败: {e}")))?;

        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            // manifest 读失败不阻塞列表（损坏条目降级为占位摘要）
            let body = es.read_entity(category, &id, manifest).await.ok();
            items.push(self.summarize(ctx, &id, body.as_deref()).await);
        }
        Ok(items)
    }

    /// 单项摘要钩子。`manifest` 为该实体主文件内容（读取失败时为 None）。
    async fn summarize(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        _manifest: Option<&str>,
    ) -> ResourceSummary {
        ResourceSummary::new(self.kind(), id, id)
    }

    /// manifest 写盘前校验/规范化钩子（默认原样放行）。
    ///
    /// 返回值是实际写盘的规范化 manifest（model 用它填充 id/name 缺省值）。
    async fn validate_manifest(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
        manifest: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(manifest.clone())
    }

    /// 写盘成功后的内存同步钩子（mcp 回灌 config，model 同步注册表，agent 失效缓存）
    async fn on_uploaded(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// 删除成功后的内存/缓存清理钩子
    async fn on_deleted(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// 连接测试/实时状态钩子（默认 NotImplemented）。
    ///
    /// 连接失败建议映射为 `Ok(status: "failed")` 而非 Err，
    /// 以便 [`dispatch`] 统一推送 resource 事件。
    async fn test_status(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<ResourceStatusResponse, PluginError> {
        Err(PluginError::NotImplemented)
    }
}

// ==================== provider 注册表 ====================

/// 资源类型（provider）注册清单 —— 宿主级单一真相源。
///
/// 语义上是"后端主动注册有哪些资源 provider"：新插件接入统一资源协议时，
/// 只需实现 [`ResourceProvider`]、在插件 route 顶部接入 [`dispatch`]，并在此
/// 登记一条 [`ResourceProviderInfo`]——前端即可自动发现该类型（生成导航、
/// 进入统一资源页），无需改动任何前端代码。
///
/// `prefix` 为资源操作路径前缀（前端拼接 `${prefix}/resources/<op>`）；
/// `supports_upload` 表示该类型在资源管理器内能否创建/删除（有无实体目录、
/// dispatch 是否实现 upload/delete——session 为 false，因其走 SessionStore 且
/// upload/delete 未实现）。
#[derive(Debug, Clone, Copy)]
pub struct ResourceProviderInfo {
    /// 资源类型（kind）
    pub kind: &'static str,
    /// 提供方显示名（路径 `[provider]/[id].[kind]`）
    pub provider_name: &'static str,
    /// 资源操作路径前缀
    pub prefix: &'static str,
    pub capabilities: &'static ResourceCapabilities,
    pub order: i32,
    /// 展示标签
    pub label: &'static str,
    pub supports_upload: bool,
    /// 列表简洁模式：仅显示类型图标 + 标题
    pub compact_list: bool,
    /// 左侧主导航归属分组（`NAV_RESOURCES` / `NAV_SETTINGS`；空串不进导航）
    pub nav: &'static str,
}

/// 全部已注册资源 provider（编译期收起当前六类，顺序即展示顺序）
pub fn provider_registry() -> &'static [ResourceProviderInfo] {
    const REG: &[ResourceProviderInfo] = &[
        ResourceProviderInfo {
            kind: RESOURCE_MODEL,
            provider_name: RESOURCE_MODEL,
            prefix: "worker/model",
            capabilities: &ResourceCapabilities::MODEL,
            order: 1,
            label: "Model Provider",
            supports_upload: true,
            compact_list: false,
            nav: NAV_RESOURCES,
        },
        ResourceProviderInfo {
            kind: RESOURCE_MCP,
            provider_name: RESOURCE_MCP,
            prefix: "mcp",
            capabilities: &ResourceCapabilities::MCP,
            order: 2,
            label: "MCP Server",
            supports_upload: true,
            compact_list: false,
            nav: NAV_RESOURCES,
        },
        ResourceProviderInfo {
            kind: RESOURCE_AGENT,
            provider_name: RESOURCE_AGENT,
            prefix: "agent",
            capabilities: &ResourceCapabilities::AGENT,
            order: 3,
            label: "Agent",
            supports_upload: true,
            compact_list: false,
            nav: NAV_RESOURCES,
        },
        ResourceProviderInfo {
            kind: RESOURCE_SKILL,
            provider_name: RESOURCE_SKILL,
            prefix: "skill",
            capabilities: &ResourceCapabilities::SKILL,
            order: 4,
            label: "Skill",
            supports_upload: true,
            compact_list: false,
            nav: NAV_RESOURCES,
        },
        ResourceProviderInfo {
            kind: RESOURCE_SESSION,
            provider_name: RESOURCE_SESSION,
            prefix: "worker/session",
            capabilities: &ResourceCapabilities::SESSION,
            order: 5,
            label: "Session",
            // session 走 SessionStore，upload/delete 未实现；
            // 不进入主导航——会话由独立的"会话"主入口承载
            supports_upload: false,
            compact_list: false,
            nav: "",
        },
        ResourceProviderInfo {
            kind: RESOURCE_SETTING,
            provider_name: RESOURCE_SETTING,
            prefix: "setting",
            capabilities: &ResourceCapabilities::SETTING,
            order: 6,
            label: "设置",
            // 设置分区清单固定，不可在资源管理器内新建/删除；
            // 各分区保存由前端 editor 自持通道完成（config/set / appearance store）。
            // 导航归属 settings：左侧"设置"入口由本条目动态驱动（不再前端写死）
            supports_upload: false,
            compact_list: true,
            nav: NAV_SETTINGS,
        },
    ];
    REG
}

/// 将静态注册表转换为可序列化的 [`ProvidersResponse`]
pub fn providers_response() -> ProvidersResponse {
    ProvidersResponse {
        providers: provider_registry()
            .iter()
            .map(|p| ProviderInfo {
                kind: p.kind.to_string(),
                provider_name: p.provider_name.to_string(),
                prefix: p.prefix.to_string(),
                capabilities: *p.capabilities,
                order: p.order,
                label: p.label.to_string(),
                supports_upload: p.supports_upload,
                compact_list: p.compact_list,
                nav: p.nav.to_string(),
            })
            .collect(),
    }
}

// ==================== 统一分发 ====================

/// `resources/*` 统一分发入口。
///
/// 返回 `None` 表示该 path 不是资源路径（插件继续自己的 match）；
/// 插件 route 顶部接入：
///
/// ```ignore
/// if let Some(resp) = crate::symbio_core::resources::dispatch(self.as_ref(), path, &ctx).await {
///     return resp;
/// }
/// ```
pub async fn dispatch<P: ResourceProvider + ?Sized>(
    provider: &P,
    path: &str,
    ctx: &Arc<dyn InvokeRequest>,
) -> Option<InvokeResponse<PluginPayload>> {
    let resp = match path {
        RESOURCES_LIST => dispatch_list(provider, ctx).await,
        RESOURCES_GET => dispatch_get(provider, ctx).await,
        RESOURCES_UPLOAD => dispatch_upload(provider, ctx).await,
        RESOURCES_DELETE => dispatch_delete(provider, ctx).await,
        RESOURCES_STATUS => dispatch_status(provider, ctx).await,
        _ => return None,
    };
    Some(resp)
}

async fn dispatch_list<P: ResourceProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let mut items = provider.list_items(ctx).await?;
    fill_provider(provider, &mut items);
    Ok(PluginPayload::new(&ResourcesListResponse {
        kind: provider.kind().to_string(),
        capabilities: capabilities_for(provider.kind()),
        items,
    }))
}

async fn dispatch_get<P: ResourceProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceGetRequest = ctx.payload()?;
    let (Some(category), Some(manifest)) = (provider.category(), provider.manifest_file()) else {
        return Err(PluginError::NotImplemented);
    };
    let store = storage_service(ctx)?;
    let es = store.entity_store();
    let content = es
        .read_entity(category, &req.id, manifest)
        .await
        .map_err(|e| PluginError::NotFound(format!("未找到资源 {}（读取失败: {e}）", req.id)))?;
    let mut item = provider.summarize(ctx, &req.id, Some(&content)).await;
    fill_provider(provider, std::slice::from_mut(&mut item));
    Ok(PluginPayload::new(&item))
}

/// 统一回填 provider 显示名（summary 未自带时填 `provider_name()`，插件零改动）
fn fill_provider<P: ResourceProvider + ?Sized>(provider: &P, items: &mut [ResourceSummary]) {
    for item in items.iter_mut() {
        if item.provider.is_none() {
            item.provider = Some(provider.provider_name().to_string());
        }
    }
}

async fn dispatch_upload<P: ResourceProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceUploadRequest = ctx.payload()?;
    let id = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PluginError::ValidationError("资源名称不能为空".to_string()))?
        .to_string();

    let Some(category) = provider.category() else {
        // 无实体目录的资源（如 session）：协议槽位，待后续导入/导出实现
        return Err(PluginError::NotImplemented);
    };

    let existed = {
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        es.entity_exists(category, &id)
            .await
            .map_err(|e| PluginError::InternalError(format!("查询资源失败: {e}")))?
    };

    if let Some(b64) = req.zip_b64.as_deref() {
        let bytes = decode_zip_b64(b64)?;
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        extract_zip_to_entity(es, category, &id, &bytes).await?;
    } else if let Some(manifest) = req.manifest.as_ref() {
        let Some(manifest_file) = provider.manifest_file() else {
            return Err(PluginError::ValidationError(
                "该资源不支持表单上传（manifest）".to_string(),
            ));
        };
        let normalized = provider.validate_manifest(ctx, &id, manifest).await?;
        let content = serde_json::to_string_pretty(&normalized)?;
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        es.write_entity(category, &id, manifest_file, &content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入资源失败: {e}")))?;
    } else {
        return Err(PluginError::ValidationError(
            "上传内容不能为空（zip_b64 或 manifest 二选一）".to_string(),
        ));
    }

    provider.on_uploaded(ctx, &id).await?;

    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: provider.kind().to_string(),
        id,
        created: !existed,
    }))
}

async fn dispatch_delete<P: ResourceProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceDeleteRequest = ctx.payload()?;
    let Some(category) = provider.category() else {
        return Err(PluginError::NotImplemented);
    };

    // 幂等删除：磁盘已无目录时仅告警，继续内存清理
    {
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        match es.delete_entity(category, &req.id).await {
            Ok(()) => {}
            Err(EntityStoreError::NotFound { .. }) => {
                crate::plugin_warn!(
                    provider.kind(),
                    "磁盘上已无资源 {} 目录，仅清理内存",
                    req.id
                );
            }
            Err(e) => {
                return Err(PluginError::InternalError(format!("删除资源失败: {e}")));
            }
        }
    }

    provider.on_deleted(ctx, &req.id).await?;

    Ok(PluginPayload::new(&ResourceUploadResponse {
        kind: provider.kind().to_string(),
        id: req.id,
        created: false,
    }))
}

async fn dispatch_status<P: ResourceProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: ResourceStatusRequest = ctx.payload()?;
    let resp = provider.test_status(ctx, &req.id).await?;

    // 连接测试能力开启时，把测试结果实时推送 resource 事件总线
    if capabilities_for(provider.kind()).test_connection {
        crate::symbio_core::event_bus::EventBus::publish_resource_status(
            provider.kind(),
            &req.id,
            &resp.status,
            resp.status_detail.clone(),
        )
        .await;
    }

    Ok(PluginPayload::new(&resp))
}

/// 统一资源操作错误（转为 PluginError::Other 抛出）
#[derive(Debug, thiserror::Error)]
#[error("resource error: {0}")]
pub struct ResourceError(pub String);

/// base64 解码 zip（上传 payload 携带 `zip_b64`）
pub fn decode_zip_b64(s: &str) -> Result<Vec<u8>, ResourceError> {
    use base64::engine::general_purpose::STANDARD;
    STANDARD
        .decode(s)
        .map_err(|e| ResourceError(format!("zip base64 解码失败: {e}")))
}

/// 解析 zip 字节为 `(相对路径, 内容)` 列表。
///
/// - 跳过目录条目、`__MACOSX` 元数据、隐藏文件
/// - 强行去掉条目前导的 `./` / `/`
pub fn parse_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ResourceError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| ResourceError(format!("非法 zip: {e}")))?;

    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ResourceError(format!("读取 zip 条目失败: {e}")))?;

        let raw = file.name().replace('\\', "/");
        if file.is_dir() {
            continue;
        }
        // 跳过 macOS 元数据 / 隐藏文件
        if raw.contains("__MACOSX")
            || raw.split('/').any(|seg| seg.starts_with('.') && !seg.is_empty())
        {
            continue;
        }
        let rel = normalize_zip_path(&raw);
        if rel.is_empty() {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| ResourceError(format!("读取 zip 条目内容失败: {e}")))?;
        out.push((rel, buf));
    }
    Ok(out)
}

/// 若 zip 内所有条目共享一个顶层根目录（常见打包方式），剥离该层，
/// 使内容平铺到目标资源目录下。
pub fn strip_common_root(entries: &mut [(String, Vec<u8>)]) {
    if entries.is_empty() {
        return;
    }
    let root_candidates: Option<String> = entries
        .iter()
        .filter_map(|(p, _)| p.split('/').next())
        .filter(|seg| !seg.is_empty())
        .min()
        .map(|s| s.to_string());
    // 仅当每个条目都以此根目录开头时才剥离
    if let Some(root) = root_candidates.as_ref() {
        let prefix = root.to_string() + "/";
        if entries.iter().all(|(p, _)| p.starts_with(&prefix)) {
            for (p, _) in entries.iter_mut() {
                if let Some(rest) = p.strip_prefix(&prefix) {
                    *p = rest.to_string();
                }
            }
        }
    }
}

/// 把已解析的 zip 内容解压写入 `EntityStore` 的 `<category>/<id>/` 目录。
///
/// - 若目录已存在则整体删除重建（上传即覆盖整包）
/// - 返回写入的文件数量
pub async fn extract_zip_to_entity(
    es: &dyn EntityStore,
    category: &str,
    id: &str,
    bytes: &[u8],
) -> Result<usize, ResourceError> {
    let mut entries = parse_zip(bytes)?;
    strip_common_root(&mut entries);
    if entries.is_empty() {
        return Err(ResourceError("zip 中没有任何可用的资源文件".to_string()));
    }

    let dir = es.entity_dir(category, id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| ResourceError(format!("清理旧资源目录失败: {e}")))?;
    }

    for (rel, content) in &entries {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ResourceError(format!("创建目录失败: {e}")))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ResourceError(format!("写入资源文件失败: {e}")))?;
    }
    Ok(entries.len())
}

/// 规范化 zip 内部相对路径文本（去掉前导 `./` 与 `/`）
fn normalize_zip_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// 自定义资源错误转 PluginError
impl From<ResourceError> for crate::symbio_core::PluginError {
    fn from(e: ResourceError) -> Self {
        crate::symbio_core::PluginError::InternalError(e.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个内存 zip（按给定顺序写入条目）
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        let mut buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn parse_zip_filters_meta_and_hidden() {
        let bytes = make_zip(&[
            ("__MACOSX/._x", b"meta"),
            (".hidden", b"y"),
            ("real.txt", b"hi"),
            ("dir/z.txt", b"z"),
        ]);
        let entries = parse_zip(&bytes).unwrap();
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["real.txt", "dir/z.txt"]);
        // 内容完整保留
        assert_eq!(entries[0].1, b"hi");
        assert_eq!(entries[1].1, b"z");
    }

    #[test]
    fn strip_common_root_peels_single_root() {
        let zip = make_zip(&[("skill/README.md", b"a"), ("skill/SKILL.md", b"b")]);
        let mut entries = parse_zip(&zip).unwrap();
        strip_common_root(&mut entries);
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["README.md", "SKILL.md"]);
    }

    #[test]
    fn strip_common_root_keeps_mixed_paths() {
        // 根目录不一致时不应剥离
        let zip = make_zip(&[("a.txt", b"a"), ("b/x.txt", b"b")]);
        let mut entries = parse_zip(&zip).unwrap();
        strip_common_root(&mut entries);
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b/x.txt"]);
    }

    #[test]
    fn zip_b64_round_trip() {
        let raw = b"zip-bytes";
        let b64 = use_base64(raw);
        let back = decode_zip_b64(&b64).unwrap();
        assert_eq!(back, raw);
    }

    fn use_base64(input: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD;
        STANDARD.encode(input)
    }

    // ==================== dispatch 单测 ====================

    use crate::symbio_core::SimpleRequest;

    /// 哑 provider：无实体目录（session 形态），仅重写 list / test_status
    struct DummyProvider {
        items: Vec<ResourceSummary>,
        status: Option<ResourceStatusResponse>,
    }

    #[async_trait]
    impl ResourceProvider for DummyProvider {
        fn kind(&self) -> &'static str {
            RESOURCE_SESSION
        }

        async fn list_items(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
        ) -> Result<Vec<ResourceSummary>, PluginError> {
            match self.items.is_empty() {
                true => Err(PluginError::InternalError("no items".to_string())),
                false => Ok(self.items.clone()),
            }
        }

        async fn test_status(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            id: &str,
        ) -> Result<ResourceStatusResponse, PluginError> {
            match &self.status {
                Some(s) => Ok(ResourceStatusResponse {
                    id: id.to_string(),
                    ..s.clone()
                }),
                None => Err(PluginError::NotImplemented),
            }
        }
    }

    fn ctx_with_payload(payload: serde_json::Value) -> Arc<dyn InvokeRequest> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set_payload(payload).unwrap();
        ctx
    }

    #[tokio::test]
    async fn dispatch_ignores_non_resource_path() {
        let p = DummyProvider {
            items: vec![],
            status: None,
        };
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        assert!(dispatch(&p, "chat/send", &ctx).await.is_none());
        assert!(dispatch(&p, "", &ctx).await.is_none());
    }

    #[tokio::test]
    async fn dispatch_list_wraps_capabilities() {
        let mut it = ResourceSummary::new(RESOURCE_SESSION, "s1", "会话一");
        it.status = "working".to_string();
        let p = DummyProvider {
            items: vec![it],
            status: None,
        };
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let resp = dispatch(&p, RESOURCES_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<ResourcesListResponse>().unwrap();
        assert_eq!(data.kind, RESOURCE_SESSION);
        assert!(data.capabilities.realtime_status);
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].id, "s1");
        assert_eq!(data.items[0].status, "working");
        // dispatch 统一回填 provider（默认与 kind 相同）
        assert_eq!(data.items[0].provider.as_deref(), Some("session"));
    }

    #[tokio::test]
    async fn dispatch_no_category_upload_delete_get_not_implemented() {
        let p = DummyProvider {
            items: vec![],
            status: None,
        };
        let up = ctx_with_payload(serde_json::json!({
            "kind": "session", "name": "x", "zip_b64": "aGk="
        }));
        assert!(matches!(
            dispatch(&p, RESOURCES_UPLOAD, &up).await,
            Some(Err(PluginError::NotImplemented))
        ));
        let del = ctx_with_payload(serde_json::json!({"kind": "session", "id": "x"}));
        assert!(matches!(
            dispatch(&p, RESOURCES_DELETE, &del).await,
            Some(Err(PluginError::NotImplemented))
        ));
        let get = ctx_with_payload(serde_json::json!({"kind": "session", "id": "x"}));
        assert!(matches!(
            dispatch(&p, RESOURCES_GET, &get).await,
            Some(Err(PluginError::NotImplemented))
        ));
    }

    #[tokio::test]
    async fn dispatch_status_ok_and_not_implemented() {
        // test_status 返回 Ok：正常返回响应（session 无 test_connection 能力，不推事件）
        let p = DummyProvider {
            items: vec![],
            status: Some(ResourceStatusResponse {
                kind: RESOURCE_SESSION.to_string(),
                id: String::new(),
                status: "working".to_string(),
                status_detail: None,
            }),
        };
        let ctx = ctx_with_payload(serde_json::json!({"kind": "session", "id": "s1"}));
        let resp = dispatch(&p, RESOURCES_STATUS, &ctx).await.unwrap().unwrap();
        let data = resp.get::<ResourceStatusResponse>().unwrap();
        assert_eq!(data.id, "s1");
        assert_eq!(data.status, "working");

        // 默认 test_status → NotImplemented
        let p2 = DummyProvider {
            items: vec![],
            status: None,
        };
        assert!(matches!(
            dispatch(&p2, RESOURCES_STATUS, &ctx).await,
            Some(Err(PluginError::NotImplemented))
        ));
    }

    #[tokio::test]
    async fn dispatch_upload_requires_name_and_content() {
        let p = DummyProvider {
            items: vec![],
            status: None,
        };
        // name 缺失 → ValidationError（在 category 检查之前）
        let ctx = ctx_with_payload(serde_json::json!({"kind": "session"}));
        assert!(matches!(
            dispatch(&p, RESOURCES_UPLOAD, &ctx).await,
            Some(Err(PluginError::ValidationError(_)))
        ));
    }
}