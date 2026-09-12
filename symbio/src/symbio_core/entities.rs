//! 统一实体门面（协议 re-export + 通用工具）
//!
//! - 协议定义见 [`schemas::entities`]（共享给各插件 route 复用）
//! - 本模块提供 zip 上传的通用解压 / 实体目录写盘工具，让 mcp / skill / agent
//!   三类插件共享同一套"zip → `~/.symbio/plugins/<category>/<id>/`"机制，避免重复实现。
//! - [`EntityProvider`] trait + [`dispatch`] 把 `entities/*` 五个操作的公共流程
//!   （列表包装、zip/manifest 上传、幂等删除、状态事件推送）收敛到核心层，
//!   各插件只实现差异化钩子（summarize / validate_manifest / on_uploaded /
//!   on_deleted / test_status）。

pub use crate::symbio_core::schemas::entities::*;

use crate::symbio_core::providers::{EntityStore, EntityStoreError, StorageService};
use crate::symbio_core::{
    create_object, InvokeRequest, InvokeRequestExt, InvokeResponse, PluginError, PluginPayload,
};
use async_trait::async_trait;
use base64::Engine;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

/// 解析当前请求的存储服务（`~/.symbio/plugins/` 基座）
///
/// 各插件 entity handler 共用此函数取 `StorageService`。
pub fn storage_service(
    ctx: &Arc<dyn InvokeRequest>,
) -> Result<Arc<dyn StorageService>, PluginError> {
    create_object::<dyn StorageService>("storage_service", ctx.clone())
        .ok_or_else(|| PluginError::InternalError("storage_service 不可用".to_string()))
}

// ==================== EntityProvider trait ====================

/// 实体提供方 trait —— 各插件实现差异化钩子，公共流程由 [`dispatch`] 承载。
///
/// ## 默认实现与重写
///
/// - 默认 `list_items` 走 `EntityStore` 枚举 + [`Self::summarize`]（适合
///   mcp / skill 等纯目录实体）；model / session / agent 等有独立数据源的
///   重写 `list_items` 接管
/// - 默认 `upload` / `delete` 由 [`dispatch`] 基于 `category` + `manifest_file`
///   完成（zip 解压或 manifest 写盘、幂等删除）；无实体目录的实体
///   （`category() == None`，如 session）返回 `NotImplemented`，为将来
///   的导入/导出留协议槽位
#[async_trait]
pub trait EntityProvider: Send + Sync {
    /// 实体类型常量（ENTITY_MODEL / ENTITY_MCP / ...）
    fn kind(&self) -> &'static str;

    /// 提供方（插件）显示名，用于前端实体路径 `[provider]/[id].[kind]` 展示。
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

    /// 列出全部实体摘要。默认：EntityStore 枚举 + [`Self::summarize`]。
    async fn list_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<EntitySummary>, PluginError> {
        let (Some(category), Some(manifest)) = (self.category(), self.manifest_file()) else {
            return Err(PluginError::NotImplemented);
        };
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        let ids = es
            .list_entities(category)
            .await
            .map_err(|e| PluginError::InternalError(format!("列出实体失败: {e}")))?;

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
    ) -> EntitySummary {
        EntitySummary::new(self.kind(), id, id)
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

    /// 读取单个实体详情（顶层 `entities/get`；容器语义走 get_container_item）。
    ///
    /// 默认实现走 EntityStore（`category()` + `manifest_file()` 提供
    /// 分类与 manifest 文件）；非实体存储型 provider（如 session 走
    /// SessionStore）重写本方法给出自己的详情读取。
    async fn get_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<EntitySummary, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 删除单个实体（磁盘/存储删除 + 由 [`dispatch_delete`] 回调 [`Self::on_deleted`]）。
    ///
    /// 默认实现：EntityStore 目录删除（`category()` 提供分类，磁盘已无目录时
    /// 幂等告警）。**非实体存储型 provider（如 session 走 SessionStore）重写
    /// 本方法**——删除能力由注册表 `capabilities.mutable` 声明，与本钩子解耦。
    async fn delete_item(&self, ctx: &Arc<dyn InvokeRequest>, id: &str) -> Result<(), PluginError> {
        let Some(category) = self.category() else {
            return Err(PluginError::NotImplemented);
        };
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        match es.delete_entity(category, id).await {
            Ok(()) => {}
            Err(EntityStoreError::NotFound { .. }) => {
                crate::plugin_warn!(self.kind(), "磁盘上已无实体 {} 目录，仅清理内存", id);
            }
            Err(e) => {
                return Err(PluginError::InternalError(format!("删除实体失败: {e}")));
            }
        }
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
    /// 以便 [`dispatch`] 统一推送 entity 事件。
    async fn test_status(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Result<EntityStatusResponse, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 详情页定义钩子（definition-driven detail）。
    ///
    /// 返回 `None` 表示该实体无定义（前端回退注册 editor / 通用面板）；
    /// `id` 为空表示请求「新建态」定义。交互不复杂的详情页据此由前端
    /// 通用渲染器（DetailForm）动态生成，见 `schemas::entities::DetailDefinition`。
    async fn detail_definition(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
    ) -> Option<crate::symbio_core::schemas::entities::DetailDefinition> {
        None
    }

    // ==================== 容器子实体（container 语义） ====================

    /// 列出容器条目内部的子实体（`entities/list` 携带 `container` 时调用）。
    ///
    /// `sub_kind` 为 `None` 时返回全部子类型（条目 `kind` 字段供前端分类，
    /// 供容器页做类别计数）；`container` 为容器条目 id（如 agent bundle id）。
    /// `parent` 仅树视图子类别（`view = "tree"`）使用：返回该父路径的下一层
    /// 子节点（懒加载，`None` = 根层）；列表视图子类别忽略此参数。
    async fn list_container_items(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _sub_kind: Option<&str>,
        _container: &str,
        _parent: Option<&str>,
    ) -> Result<Vec<EntitySummary>, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 读取容器子实体详情；文件内容置于 `extra.content`。
    async fn get_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
        _container: &str,
    ) -> Result<EntitySummary, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 写入（创建/覆盖）容器子实体；`id` 为容器内相对路径。
    async fn put_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
        _content: &str,
        _container: &str,
    ) -> Result<EntityUploadResponse, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 删除容器子实体。
    async fn delete_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _id: &str,
        _container: &str,
    ) -> Result<EntityUploadResponse, PluginError> {
        Err(PluginError::NotImplemented)
    }

    /// 订阅容器子实体数据变更（树视图等实时场景；前端视图挂载时调用，
    /// 卸载时经 unwatch_container 配对取消）。变更经粗粒度 `data` 事件下发
    /// （kind = provider kind、sessionId = 容器 id）。默认 no-op：无实时
    /// 能力的 provider 直接成功。
    async fn watch_container(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _sub_kind: Option<&str>,
        _container: &str,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    /// 取消容器子实体数据变更订阅（与 watch_container 配对；默认 no-op）。
    async fn unwatch_container(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        _sub_kind: Option<&str>,
        _container: &str,
    ) -> Result<(), PluginError> {
        Ok(())
    }
}

// ==================== 容器子实体声明 ====================

/// 容器子实体声明（编译期静态版，下发给前端时转为
/// [`ContainerKindInfo`](crate::symbio_core::schemas::entities::ContainerKindInfo)）
#[derive(Debug, Clone, Copy)]
pub struct ContainerKindSpec {
    /// 子实体类型（如 `prompt` / `skill` / `mcp`）
    pub kind: &'static str,
    /// 展示标签
    pub label: &'static str,
    /// 语义说明（前端新建/编辑表单提示文本）
    pub description: &'static str,
    /// 新建路径模板（`<name>` 占位符），如 `prompts/<name>.md`；空 = 不可用户创建
    pub path_hint: &'static str,
    /// 新建内容模板
    pub default_content: &'static str,
    pub capabilities: &'static EntityCapabilities,
    /// 中栏展示形态：`"list"`（缺省）= 列表；`"tree"` = 树视图（懒加载，
    /// 条目携带 parent 层级）
    pub view: &'static str,
}

/// `view` 字段的"列表"取值（ContainerKindSpec 显式声明，下发时缺省省略）
pub const VIEW_LIST: &str = "list";
/// `view` 字段的"树视图"取值
pub const VIEW_TREE: &str = "tree";

/// agent bundle 内部托管的三类文件级子实体（布局与 OAB 装配规则严格一致，
/// 模板中的 frontmatter priority 约定与装配缺省值对应）。
pub static AGENT_CONTAINER_KINDS: &[ContainerKindSpec] = &[
    ContainerKindSpec {
        kind: "prompt",
        label: "提示词",
        description: "Markdown 片段，无条件追加进系统提示词；可用 YAML frontmatter 设置 priority（缺省 10，小者优先）。",
        path_hint: "prompts/<name>.md",
        default_content: "---\npriority: 10\n---\n\n在此撰写常驻系统提示词（人格 / 全局规则 / 工作流）…",
        capabilities: &EntityCapabilities::BUNDLE_FILE,
        view: VIEW_LIST,
    },
    ContainerKindSpec {
        kind: "skill",
        label: "技能",
        description: "skills/<name>/SKILL.md，正文作为提示词片段；frontmatter priority 缺省 50。",
        path_hint: "skills/<name>/SKILL.md",
        default_content: "---\npriority: 50\n---\n\n# 技能名称\n\n描述该技能的适用场景、输入输出与执行步骤…",
        capabilities: &EntityCapabilities::BUNDLE_FILE,
        view: VIEW_LIST,
    },
    ContainerKindSpec {
        kind: "mcp",
        label: "MCP",
        description: "MCP server 配置（YAML），是工具的唯一来源，原样透传给宿主 MCP 客户端。",
        path_hint: "mcps/<name>.yaml",
        default_content: "# MCP server 配置（YAML，原样透传给宿主 MCP 客户端）\ncommand: \"\"\nargs: []\nenv: {}",
        capabilities: &EntityCapabilities::BUNDLE_FILE,
        view: VIEW_LIST,
    },
];

/// session 的容器子实体声明：条目（会话）内部托管**子会话**。
///
/// 会话（session）的容器子实体声明：条目（会话）内部托管两类子实体。
///
/// - **子会话**（tree 机制之外的列表视图）：由父会话派生（系统管理，
///   非用户新建），`path_hint` 为空 = 不可用户创建，仅支持查看与删除；
///   存储由文件后端路由到父会话目录的 `sessions/` 子目录（归属声明
///   `metadata.parent_session_id`），删除父会话级联删除子会话。
/// - **目录树**（tree 机制的一个场景实现）：会话工作目录的层级浏览，
///   `view = "tree"` + 懒加载（`parent` 请求参数逐层下发），只读。
///   目录树只是 tree 机制下的一个 provider 场景——机制本身只定义
///   「层级 + 懒加载 + 选择」，不含任何文件系统语义。
pub static SESSION_CONTAINER_KINDS: &[ContainerKindSpec] = &[
    ContainerKindSpec {
        kind: ENTITY_SESSION,
        label: "子会话",
        description: "由该会话派生的子会话；随父会话级联删除。",
        path_hint: "",
        default_content: "",
        capabilities: &EntityCapabilities::SUB_SESSION,
        view: VIEW_LIST,
    },
    ContainerKindSpec {
        kind: "dir",
        label: "目录树",
        description: "会话工作目录的层级浏览（文件可查看/编辑，实时刷新）。",
        path_hint: "",
        default_content: "",
        capabilities: &EntityCapabilities::BUNDLE_FILE,
        view: VIEW_TREE,
    },
];

/// 某 provider kind 的容器子实体声明（空 = 条目不是容器）。
pub fn container_kinds_for(kind: &str) -> &'static [ContainerKindSpec] {
    match kind {
        ENTITY_AGENT => AGENT_CONTAINER_KINDS,
        ENTITY_SESSION => SESSION_CONTAINER_KINDS,
        _ => &[],
    }
}

// ==================== provider 注册表 ====================

/// 实体类型（provider）注册清单 —— 宿主级单一真相源。
///
/// 语义上是"后端主动注册有哪些实体 provider"：新插件接入统一实体协议时，
/// 只需实现 [`EntityProvider`]、在插件 route 顶部接入 [`dispatch`]，并在此
/// 登记一条 [`EntityProviderInfo`]——前端即可自动发现该类型（生成导航、
/// 进入统一实体页），无需改动任何前端代码。
///
/// `prefix` 为实体操作路径前缀（前端拼接 `${prefix}/entities/<op>`）；
/// `supports_upload` 表示该类型在实体管理器内能否创建/删除（有无实体目录、
/// dispatch 是否实现 upload/delete——session 为 false，因其走 SessionStore 且
/// upload/delete 未实现）。
#[derive(Debug, Clone, Copy)]
pub struct EntityProviderInfo {
    /// 实体类型（kind）
    pub kind: &'static str,
    /// 提供方显示名（路径 `[provider]/[id].[kind]`）
    pub provider_name: &'static str,
    /// 实体操作路径前缀
    pub prefix: &'static str,
    pub capabilities: &'static EntityCapabilities,
    pub order: i32,
    /// 展示标签
    pub label: &'static str,
    pub supports_upload: bool,
    /// 列表简洁模式：仅显示类型图标 + 标题
    pub compact_list: bool,
    /// 列表项是否显示运行状态图示（如设置分区为 false，不显示状态点）
    pub status_indicator: bool,
    /// 容器子实体声明（空 = 条目不是容器）
    pub container_kinds: &'static [ContainerKindSpec],
}

/// 全部已注册实体 provider（编译期收起当前六类，顺序即展示顺序）
/// 默认顺序：会话 / 模型 / 智能体 / 技能 / MCP / 设置。
/// 可通过配置 `symbio.provider_order` 覆盖（见 [`providers_response_with_overrides`]）。
pub fn provider_registry() -> &'static [EntityProviderInfo] {
    const REG: &[EntityProviderInfo] = &[
        EntityProviderInfo {
            kind: ENTITY_SESSION,
            provider_name: ENTITY_SESSION,
            prefix: "worker/session",
            capabilities: &EntityCapabilities::SESSION,
            order: 1,
            label: "会话",
            // session 走 SessionStore（非 EntityStore）：zip/manifest 上传不适用；
            // 删除经重写 delete_item 钩子接入统一协议；创建走前端专属 editor 引导。
            // 条目是容器：内部托管子会话（SESSION_CONTAINER_KINDS，path_hint 空
            // = 不可用户创建，仅查看/删除）
            supports_upload: false,
            compact_list: false,
            status_indicator: true,
            container_kinds: SESSION_CONTAINER_KINDS,
        },
        EntityProviderInfo {
            kind: ENTITY_MODEL,
            provider_name: ENTITY_MODEL,
            prefix: "worker/model",
            capabilities: &EntityCapabilities::MODEL,
            order: 2,
            label: "模型",
            supports_upload: true,
            compact_list: false,
            status_indicator: true,
            container_kinds: &[],
        },
        EntityProviderInfo {
            kind: ENTITY_AGENT,
            provider_name: ENTITY_AGENT,
            prefix: "agent",
            capabilities: &EntityCapabilities::AGENT,
            order: 3,
            label: "智能体",
            supports_upload: true,
            compact_list: false,
            status_indicator: true,
            // agent 条目（OAB bundle）是容器：内部托管 prompt / skill / mcp 三类文件级子实体
            container_kinds: AGENT_CONTAINER_KINDS,
        },
        EntityProviderInfo {
            kind: ENTITY_SKILL,
            provider_name: ENTITY_SKILL,
            prefix: "skill",
            capabilities: &EntityCapabilities::SKILL,
            order: 4,
            label: "技能",
            supports_upload: true,
            compact_list: false,
            status_indicator: true,
            container_kinds: &[],
        },
        EntityProviderInfo {
            kind: ENTITY_MCP,
            provider_name: ENTITY_MCP,
            prefix: "mcp",
            capabilities: &EntityCapabilities::MCP,
            order: 5,
            label: "MCP",
            supports_upload: true,
            compact_list: false,
            status_indicator: true,
            container_kinds: &[],
        },
        EntityProviderInfo {
            kind: ENTITY_SETTING,
            provider_name: ENTITY_SETTING,
            prefix: "setting",
            capabilities: &EntityCapabilities::SETTING,
            order: 6,
            label: "设置",
            // 设置分区清单固定，不可在实体管理器内新建/删除；
            // 各分区保存由前端 editor 自持通道完成（config/set / appearance store）。
            // compact_list + 无状态：列表仅显图标 + 标题，不显示状态点
            supports_upload: false,
            compact_list: true,
            status_indicator: false,
            container_kinds: &[],
        },
    ];
    REG
}

/// 将静态注册表转换为可序列化的 [`ProvidersResponse`]
///
/// 默认顺序直接来自注册表（`order` 字段），不应用任何覆盖。
pub fn providers_response() -> ProvidersResponse {
    providers_response_with_overrides(&HashMap::new())
}

/// 应用**服务器端顺序覆盖**后的 provider 响应。
///
/// `order_override` 为 `kind → order` 映射（如来自用户可配置的
/// `symbio.provider_order`）；命中覆盖的 kind 以其覆盖值为准，
/// 未命中的用注册表默认 `order`。据此重排数组并重写各 provider 的
/// `order` 字段，前端无需改动（本就按下发的 order 排序）。
pub fn providers_response_with_overrides(
    order_override: &HashMap<String, i32>,
) -> ProvidersResponse {
    let mut providers: Vec<ProviderInfo> = provider_registry()
        .iter()
        .map(|p| ProviderInfo {
            kind: p.kind.to_string(),
            provider_name: p.provider_name.to_string(),
            prefix: p.prefix.to_string(),
            capabilities: *p.capabilities,
            order: order_override.get(p.kind).copied().unwrap_or(p.order),
            label: p.label.to_string(),
            supports_upload: p.supports_upload,
            compact_list: p.compact_list,
            status_indicator: p.status_indicator,
            container_kinds: p
                .container_kinds
                .iter()
                .map(|k| ContainerKindInfo {
                    kind: k.kind.to_string(),
                    label: k.label.to_string(),
                    description: Some(k.description.to_string()),
                    path_hint: Some(k.path_hint.to_string()),
                    default_content: Some(k.default_content.to_string()),
                    capabilities: *k.capabilities,
                    view: (k.view == VIEW_TREE).then(|| VIEW_TREE.to_string()),
                })
                .collect(),
        })
        .collect();

    providers.sort_by_key(|p| p.order);
    ProvidersResponse { providers }
}

// ==================== 统一分发 ====================

/// 取非空字符串字段（`None` 或空白视为缺省）
fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `entities/*` 统一分发入口。
///
/// 返回 `None` 表示该 path 不是实体路径（插件继续自己的 match）；
/// 插件 route 顶部接入：
///
/// ```ignore
/// if let Some(resp) = crate::symbio_core::entities::dispatch(self.as_ref(), path, &ctx).await {
///     return resp;
/// }
/// ```
///
/// 各操作均支持**容器语义**：请求携带 `container`（容器条目 id，如 agent bundle id）
/// 时走容器子实体分支（[`EntityProvider`] 的 `*_container_item` 钩子），
/// 响应形状与顶层语义一致——前端用同一套服务函数与类型访问两层结构。
pub async fn dispatch<P: EntityProvider + ?Sized>(
    provider: &P,
    path: &str,
    ctx: &Arc<dyn InvokeRequest>,
) -> Option<InvokeResponse<PluginPayload>> {
    let resp = match path {
        ENTITIES_LIST => dispatch_list(provider, ctx).await,
        ENTITIES_GET => dispatch_get(provider, ctx).await,
        ENTITIES_UPLOAD => dispatch_upload(provider, ctx).await,
        ENTITIES_DELETE => dispatch_delete(provider, ctx).await,
        ENTITIES_STATUS => dispatch_status(provider, ctx).await,
        ENTITIES_DETAIL => dispatch_detail(provider, ctx).await,
        ENTITIES_WATCH => dispatch_watch(provider, ctx, true).await,
        ENTITIES_UNWATCH => dispatch_watch(provider, ctx, false).await,
        _ => return None,
    };
    Some(resp)
}

async fn dispatch_list<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    // 容器语义（payload 可缺省，容忍空请求体）
    let req = ctx
        .payload::<serde_json::Value>()
        .ok()
        .and_then(|v| serde_json::from_value::<EntitiesListRequest>(v).ok())
        .unwrap_or_default();
    if let Some(container) = non_empty(&req.container) {
        let sub_kind = req
            .sub_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let parent = req
            .parent
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut items = provider
            .list_container_items(ctx, sub_kind, container, parent)
            .await?;
        fill_provider(provider, &mut items);
        let kind = sub_kind.unwrap_or(provider.kind()).to_string();
        // 能力开关取容器声明：命中子类型取该子类声明；混合列表（未指定
        // sub_kind）= 容器文件语义（BUNDLE_FILE）；未知子类型回退 provider 能力
        let capabilities = container_kinds_for(provider.kind())
            .iter()
            .find(|k| k.kind == kind)
            .map(|k| *k.capabilities)
            .unwrap_or_else(|| {
                if sub_kind.is_none() {
                    EntityCapabilities::BUNDLE_FILE
                } else {
                    capabilities_for(provider.kind())
                }
            });
        return Ok(PluginPayload::new(&EntitiesListResponse {
            kind,
            capabilities,
            items,
            container: Some(container.to_string()),
        }));
    }

    let mut items = provider.list_items(ctx).await?;
    fill_provider(provider, &mut items);
    Ok(PluginPayload::new(&EntitiesListResponse {
        kind: provider.kind().to_string(),
        capabilities: capabilities_for(provider.kind()),
        items,
        container: None,
    }))
}

async fn dispatch_get<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: EntityGetRequest = ctx.payload()?;
    // 容器语义：读取容器子实体详情（内容在 extra.content）
    if let Some(container) = non_empty(&req.container) {
        let mut item = provider.get_container_item(ctx, &req.id, container).await?;
        fill_provider(provider, std::slice::from_mut(&mut item));
        return Ok(PluginPayload::new(&item));
    }
    // 实体存储型 provider：走 EntityStore 读取；否则回退 provider 的
    // 顶级 get 钩子（如 session 走 SessionStore）。
    match (provider.category(), provider.manifest_file()) {
        (Some(category), Some(manifest)) => {
            let store = storage_service(ctx)?;
            let es = store.entity_store();
            let content = es
                .read_entity(category, &req.id, manifest)
                .await
                .map_err(|e| {
                    PluginError::NotFound(format!("未找到实体 {}（读取失败: {e}）", req.id))
                })?;
            let mut item = provider.summarize(ctx, &req.id, Some(&content)).await;
            fill_provider(provider, std::slice::from_mut(&mut item));
            Ok(PluginPayload::new(&item))
        }
        _ => {
            let mut item = provider.get_item(ctx, &req.id).await?;
            fill_provider(provider, std::slice::from_mut(&mut item));
            Ok(PluginPayload::new(&item))
        }
    }
}

/// 统一回填 provider 显示名（summary 未自带时填 `provider_name()`，插件零改动）
fn fill_provider<P: EntityProvider + ?Sized>(provider: &P, items: &mut [EntitySummary]) {
    for item in items.iter_mut() {
        if item.provider.is_none() {
            item.provider = Some(provider.provider_name().to_string());
        }
    }
}

async fn dispatch_upload<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: EntityUploadRequest = ctx.payload()?;
    // 容器语义：manifest.content 即文件内容，name 即容器内相对路径（创建/覆盖）
    if let Some(container) = non_empty(&req.container) {
        let path = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| PluginError::ValidationError("容器子实体路径不能为空".to_string()))?;
        let content = req
            .manifest
            .as_ref()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                PluginError::ValidationError(
                    "容器子实体写入需要 manifest.content（文件文本）".to_string(),
                )
            })?;
        let resp = provider
            .put_container_item(ctx, path, content, container)
            .await?;
        return Ok(PluginPayload::new(&resp));
    }

    let id = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PluginError::ValidationError("实体名称不能为空".to_string()))?
        .to_string();

    let Some(category) = provider.category() else {
        // 无实体目录的实体（如 session）：协议槽位，待后续导入/导出实现
        return Err(PluginError::NotImplemented);
    };

    let existed = {
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        es.entity_exists(category, &id)
            .await
            .map_err(|e| PluginError::InternalError(format!("查询实体失败: {e}")))?
    };

    if let Some(b64) = req.zip_b64.as_deref() {
        let bytes = decode_zip_b64(b64)?;
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        extract_zip_to_entity(es, category, &id, &bytes).await?;
    } else if let Some(manifest) = req.manifest.as_ref() {
        let Some(manifest_file) = provider.manifest_file() else {
            return Err(PluginError::ValidationError(
                "该实体不支持表单上传（manifest）".to_string(),
            ));
        };
        let normalized = provider.validate_manifest(ctx, &id, manifest).await?;
        let content = serde_json::to_string_pretty(&normalized)?;
        let store = storage_service(ctx)?;
        let es = store.entity_store();
        es.write_entity(category, &id, manifest_file, &content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入实体失败: {e}")))?;
    } else {
        return Err(PluginError::ValidationError(
            "上传内容不能为空（zip_b64 或 manifest 二选一）".to_string(),
        ));
    }

    provider.on_uploaded(ctx, &id).await?;

    // 实体生命周期变更通知：前端据此即时同步清单（created 乐观插入 / updated 重拉）。
    // 事件总线不可用不应影响上传本身的结果。上传实体均为顶层（parent_id = None）。
    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        provider.kind(),
        &id,
        if existed { "updated" } else { "created" },
        None,
        None,
    )
    .await;

    Ok(PluginPayload::new(&EntityUploadResponse {
        kind: provider.kind().to_string(),
        id,
        created: !existed,
    }))
}

async fn dispatch_delete<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: EntityDeleteRequest = ctx.payload()?;
    // 容器语义：删除容器子实体（幂等语义由插件钩子决定）
    if let Some(container) = non_empty(&req.container) {
        let resp = provider
            .delete_container_item(ctx, &req.id, container)
            .await?;
        return Ok(PluginPayload::new(&resp));
    }

    // 删除统一走可覆盖钩子（EntityStore 默认实现 / 非实体存储型 provider 重写）
    provider.delete_item(ctx, &req.id).await?;

    provider.on_deleted(ctx, &req.id).await?;

    // 实体生命周期变更通知：前端据此即时把该项从清单中移除。
    // 容器子实体删除（上方提前 return）暂不通知——前端容器文件树由
    // 容器实体自身的 entities/list 刷新。此路径删除的是顶层实体（parent_id = None）。
    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        provider.kind(),
        &req.id,
        "deleted",
        None,
        None,
    )
    .await;

    Ok(PluginPayload::new(&EntityUploadResponse {
        kind: provider.kind().to_string(),
        id: req.id,
        created: false,
    }))
}

async fn dispatch_status<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: EntityStatusRequest = ctx.payload()?;
    let resp = provider.test_status(ctx, &req.id).await?;

    // 连接测试能力开启时，把测试结果实时推送 entity 事件总线
    if capabilities_for(provider.kind()).test_connection {
        crate::symbio_core::event_bus::EventBus::publish_entity_status(
            provider.kind(),
            &req.id,
            &resp.status,
            resp.status_detail.clone(),
        )
        .await;
    }

    Ok(PluginPayload::new(&resp))
}

/// `entities/detail`：详情页定义下发（definition-driven detail）。
///
/// `id` 为空 = 「新建态」定义；provider 未实现钩子时 `definition = None`，
/// 前端回退注册 editor / 通用兜底面板。
async fn dispatch_detail<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: crate::symbio_core::schemas::entities::DetailDefinitionRequest = ctx.payload()?;
    let definition = provider.detail_definition(ctx, &req.id).await;
    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::entities::DetailDefinitionResponse { definition },
    ))
}

/// 容器数据变更订阅/取消（watch = true 订阅，false 取消；载荷复用
/// EntitiesListRequest 的 container + sub_kind 字段）
async fn dispatch_watch<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
    watch: bool,
) -> InvokeResponse<PluginPayload> {
    let req = ctx
        .payload::<EntitiesListRequest>()
        .ok()
        .unwrap_or_default();
    let container = non_empty(&req.container)
        .ok_or_else(|| PluginError::ValidationError("watch 需要 container".into()))?;
    let sub_kind = req
        .sub_kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if watch {
        provider.watch_container(ctx, sub_kind, container).await?;
    } else {
        provider.unwatch_container(ctx, sub_kind, container).await?;
    }
    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::common::SuccessResponse::default(),
    ))
}

/// 统一实体操作错误（转为 PluginError::Other 抛出）
#[derive(Debug, thiserror::Error)]
#[error("entity error: {0}")]
pub struct EntityError(pub String);

/// base64 解码 zip（上传 payload 携带 `zip_b64`）
pub fn decode_zip_b64(s: &str) -> Result<Vec<u8>, EntityError> {
    use base64::engine::general_purpose::STANDARD;
    STANDARD
        .decode(s)
        .map_err(|e| EntityError(format!("zip base64 解码失败: {e}")))
}

/// 解析 zip 字节为 `(相对路径, 内容)` 列表。
///
/// - 跳过目录条目、`__MACOSX` 元数据、隐藏文件
/// - 强行去掉条目前导的 `./` / `/`
pub fn parse_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, EntityError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| EntityError(format!("非法 zip: {e}")))?;

    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| EntityError(format!("读取 zip 条目失败: {e}")))?;

        let raw = file.name().replace('\\', "/");
        if file.is_dir() {
            continue;
        }
        // 跳过 macOS 元数据 / 隐藏文件
        if raw.contains("__MACOSX")
            || raw
                .split('/')
                .any(|seg| seg.starts_with('.') && !seg.is_empty())
        {
            continue;
        }
        let rel = normalize_zip_path(&raw);
        if rel.is_empty() {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| EntityError(format!("读取 zip 条目内容失败: {e}")))?;
        out.push((rel, buf));
    }
    Ok(out)
}

/// 若 zip 内所有条目共享一个顶层根目录（常见打包方式），剥离该层，
/// 使内容平铺到目标实体目录下。
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
) -> Result<usize, EntityError> {
    let mut entries = parse_zip(bytes)?;
    strip_common_root(&mut entries);
    if entries.is_empty() {
        return Err(EntityError("zip 中没有任何可用的实体文件".to_string()));
    }

    let dir = es.entity_dir(category, id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| EntityError(format!("清理旧实体目录失败: {e}")))?;
    }

    for (rel, content) in &entries {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| EntityError(format!("创建目录失败: {e}")))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| EntityError(format!("写入实体文件失败: {e}")))?;
    }
    Ok(entries.len())
}

/// 规范化 zip 内部相对路径文本（去掉前导 `./` 与 `/`）
fn normalize_zip_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// 自定义实体错误转 PluginError
impl From<EntityError> for crate::symbio_core::PluginError {
    fn from(e: EntityError) -> Self {
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
    fn providers_response_applies_order_override() {
        use std::collections::HashMap;

        let mut overrides = HashMap::new();
        // 把 setting 提到最前、model 压到最后
        overrides.insert(ENTITY_SETTING.to_string(), -10);
        overrides.insert(ENTITY_MODEL.to_string(), 100);

        let resp = providers_response_with_overrides(&overrides);
        let kinds: Vec<&str> = resp.providers.iter().map(|p| p.kind.as_str()).collect();

        // 覆盖后的顺序：setting 最前、model 最后，其余保持注册表相对序
        assert_eq!(kinds.first(), Some(&ENTITY_SETTING));
        assert_eq!(kinds.last(), Some(&ENTITY_MODEL));
        assert!(
            resp.providers.windows(2).all(|w| w[0].order <= w[1].order),
            "order 应单调不减"
        );
        // 覆盖值确实写回各 provider 的 order
        let setting = resp
            .providers
            .iter()
            .find(|p| p.kind == ENTITY_SETTING)
            .unwrap();
        assert_eq!(setting.order, -10);
    }

    #[test]
    fn providers_response_default_order_without_override() {
        let resp = providers_response();
        // 无覆盖时严格等于注册表顺序：会话/模型/智能体/技能/MCP/设置
        let kinds: Vec<&str> = resp.providers.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                ENTITY_SESSION,
                ENTITY_MODEL,
                ENTITY_AGENT,
                ENTITY_SKILL,
                ENTITY_MCP,
                ENTITY_SETTING,
            ]
        );
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
        items: Vec<EntitySummary>,
        status: Option<EntityStatusResponse>,
    }

    #[async_trait]
    impl EntityProvider for DummyProvider {
        fn kind(&self) -> &'static str {
            ENTITY_SESSION
        }

        async fn list_items(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
        ) -> Result<Vec<EntitySummary>, PluginError> {
            match self.items.is_empty() {
                true => Err(PluginError::InternalError("no items".to_string())),
                false => Ok(self.items.clone()),
            }
        }

        async fn test_status(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            id: &str,
        ) -> Result<EntityStatusResponse, PluginError> {
            match &self.status {
                Some(s) => Ok(EntityStatusResponse {
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
    async fn dispatch_ignores_non_entity_path() {
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
        let mut it = EntitySummary::new(ENTITY_SESSION, "s1", "会话一");
        it.status = "working".to_string();
        let p = DummyProvider {
            items: vec![it],
            status: None,
        };
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let resp = dispatch(&p, ENTITIES_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntitiesListResponse>().unwrap();
        assert_eq!(data.kind, ENTITY_SESSION);
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
            dispatch(&p, ENTITIES_UPLOAD, &up).await,
            Some(Err(PluginError::NotImplemented))
        ));
        let del = ctx_with_payload(serde_json::json!({"kind": "session", "id": "x"}));
        assert!(matches!(
            dispatch(&p, ENTITIES_DELETE, &del).await,
            Some(Err(PluginError::NotImplemented))
        ));
        let get = ctx_with_payload(serde_json::json!({"kind": "session", "id": "x"}));
        assert!(matches!(
            dispatch(&p, ENTITIES_GET, &get).await,
            Some(Err(PluginError::NotImplemented))
        ));
    }

    #[tokio::test]
    async fn dispatch_status_ok_and_not_implemented() {
        // test_status 返回 Ok：正常返回响应（session 无 test_connection 能力，不推事件）
        let p = DummyProvider {
            items: vec![],
            status: Some(EntityStatusResponse {
                kind: ENTITY_SESSION.to_string(),
                id: String::new(),
                status: "working".to_string(),
                status_detail: None,
            }),
        };
        let ctx = ctx_with_payload(serde_json::json!({"kind": "session", "id": "s1"}));
        let resp = dispatch(&p, ENTITIES_STATUS, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntityStatusResponse>().unwrap();
        assert_eq!(data.id, "s1");
        assert_eq!(data.status, "working");

        // 默认 test_status → NotImplemented
        let p2 = DummyProvider {
            items: vec![],
            status: None,
        };
        assert!(matches!(
            dispatch(&p2, ENTITIES_STATUS, &ctx).await,
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
            dispatch(&p, ENTITIES_UPLOAD, &ctx).await,
            Some(Err(PluginError::ValidationError(_)))
        ));
    }

    // ==================== 容器语义 dispatch 单测 ====================

    /// 容器 provider：实现容器四钩子（模拟 agent bundle 内部实体）
    struct ContainerProvider;

    #[async_trait]
    impl EntityProvider for ContainerProvider {
        fn kind(&self) -> &'static str {
            ENTITY_AGENT
        }

        async fn list_container_items(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            sub_kind: Option<&str>,
            container: &str,
            _parent: Option<&str>,
        ) -> Result<Vec<EntitySummary>, PluginError> {
            Ok(["prompt", "skill"]
                .into_iter()
                .filter(|k| sub_kind.is_none_or(|sk| *k == sk))
                .map(|k| EntitySummary::new(k, format!("{container}/{k}.md"), k))
                .collect())
        }

        async fn get_container_item(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            id: &str,
            _container: &str,
        ) -> Result<EntitySummary, PluginError> {
            let mut it = EntitySummary::new("prompt", id, id);
            it.extra = serde_json::json!({ "content": "hello" });
            Ok(it)
        }

        async fn put_container_item(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            id: &str,
            _content: &str,
            _container: &str,
        ) -> Result<EntityUploadResponse, PluginError> {
            Ok(EntityUploadResponse {
                kind: "prompt".into(),
                id: id.to_string(),
                created: true,
            })
        }

        async fn delete_container_item(
            &self,
            _ctx: &Arc<dyn InvokeRequest>,
            id: &str,
            _container: &str,
        ) -> Result<EntityUploadResponse, PluginError> {
            Ok(EntityUploadResponse {
                kind: "prompt".into(),
                id: id.to_string(),
                created: false,
            })
        }
    }

    #[tokio::test]
    async fn dispatch_container_list_get_put_delete() {
        let p = ContainerProvider;

        // list（带 container）：响应回显 container，能力取容器声明（BUNDLE_FILE 可写）
        let ctx = ctx_with_payload(serde_json::json!({"container": "com.acme"}));
        let resp = dispatch(&p, ENTITIES_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntitiesListResponse>().unwrap();
        assert_eq!(data.container.as_deref(), Some("com.acme"));
        assert_eq!(data.kind, ENTITY_AGENT);
        assert!(data.capabilities.mutable && !data.capabilities.zip_upload);
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].kind, "prompt");

        // list（带 container + sub_kind）：能力取子类型声明，kind 为子类型
        let ctx =
            ctx_with_payload(serde_json::json!({"container": "com.acme", "sub_kind": "prompt"}));
        let resp = dispatch(&p, ENTITIES_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntitiesListResponse>().unwrap();
        assert_eq!(data.kind, "prompt");
        assert_eq!(data.items.len(), 1);

        // get（带 container）：内容在 extra.content
        let ctx = ctx_with_payload(
            serde_json::json!({"kind": "agent", "id": "prompts/a.md", "container": "com.acme"}),
        );
        let resp = dispatch(&p, ENTITIES_GET, &ctx).await.unwrap().unwrap();
        let item = resp.get::<EntitySummary>().unwrap();
        assert_eq!(
            item.extra.get("content").and_then(|c| c.as_str()),
            Some("hello")
        );
        assert_eq!(item.provider.as_deref(), Some(ENTITY_AGENT));

        // put（带 container）：name 为路径，manifest.content 为内容
        let ctx = ctx_with_payload(serde_json::json!({
            "kind": "agent", "name": "prompts/b.md",
            "manifest": {"content": "x"}, "container": "com.acme"
        }));
        let resp = dispatch(&p, ENTITIES_UPLOAD, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntityUploadResponse>().unwrap();
        assert_eq!(data.id, "prompts/b.md");
        assert!(data.created);

        // delete（带 container）
        let ctx = ctx_with_payload(
            serde_json::json!({"kind": "agent", "id": "prompts/a.md", "container": "com.acme"}),
        );
        let resp = dispatch(&p, ENTITIES_DELETE, &ctx).await.unwrap().unwrap();
        let data = resp.get::<EntityUploadResponse>().unwrap();
        assert_eq!(data.id, "prompts/a.md");
    }

    #[tokio::test]
    async fn dispatch_container_without_hook_is_not_implemented() {
        // 顶层 provider 未实现容器钩子：带 container 的请求应报 NotImplemented（而非静默走顶层语义）
        let p = DummyProvider {
            items: vec![],
            status: None,
        };
        let ctx =
            ctx_with_payload(serde_json::json!({"kind": "session", "id": "s1", "container": "c1"}));
        assert!(matches!(
            dispatch(&p, ENTITIES_GET, &ctx).await,
            Some(Err(PluginError::NotImplemented))
        ));
    }
}
