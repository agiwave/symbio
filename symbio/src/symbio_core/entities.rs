//! 实体机制（**后端内部抽象**）—— VDFS 的资源存储层
//!
//! ## 现状（S11 之后）
//!
//! `entities/*` **调用协议已下线**：不再有任何插件路由它，前端与 LLM 也都不使用。
//! 资源访问统一经 VDFS（`.vdfs/…`），由 `vdfs::EntityVdfsAdapter` 把本模块的
//! [`EntityProvider`] trait 适配成挂载点——**同一批资源的同一份实现**，只是不再
//! 暴露第二套对外地址。
//!
//! 本模块因此只剩两件事：
//!
//! - [`EntityProvider`] trait：各插件实现差异化钩子（list_items / summarize /
//!   validate_manifest / on_uploaded / on_deleted / test_status / 容器子实体 …）；
//! - [`entity_write`] / [`entity_delete`]：写盘与删除的**唯一实现**，供 VDFS
//!   适配器调用。
//!
//! [`entity_write`]: crate::symbio_core::entities::entity_write
//! [`entity_delete`]: crate::symbio_core::entities::entity_delete

pub use crate::symbio_core::schemas::entities::*;

use crate::symbio_core::providers::{EntityStoreError, StorageService};
use crate::symbio_core::{create_object, InvokeRequest, PluginError};
use async_trait::async_trait;
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

/// `test_status` 的结果状态：**连通**（测试通过）
pub const ENTITY_STATUS_CONNECTED: &str = "connected";
/// `test_status` 的结果状态：**失败**（测试未通过；原因见 `status_detail`）
pub const ENTITY_STATUS_FAILED: &str = "failed";

/// 实体提供方 trait —— 各插件实现差异化钩子。
///
/// **不对外暴露**：本 trait 的唯一消费者是 `vdfs::EntityVdfsAdapter`
/// （`entities/*` 协议已随 S11 下线），因此它描述的是「资源怎么存、怎么校验」，
/// 而不是「外部怎么访问」。
///
/// ## 默认实现与重写
///
/// - 默认 `list_items` 走 `EntityStore` 枚举 + [`Self::summarize`]（适合
///   mcp / skill 等纯目录实体）；model / session / agent 等有独立数据源的
///   重写 `list_items` 接管
/// - 写盘 / 删除由 [`entity_write`] / [`entity_delete`] 基于 `category` +
///   `manifest_file` 完成（manifest 写盘 + 幂等删除）；无实体目录的实体
///   （`category() == None`，如 session）返回 `NotImplemented`
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

    /// VDFS `write { create }` 的**最小落盘 manifest**。
    ///
    /// 两条链路的「新建」语义不同，故由插件自持：
    /// - **实体机制**：前端渲染完整表单，字段齐全后一次 `entities/upload`；
    /// - **VDFS**：`vdfs/write { create: true }` 只带路径名，语义是「先落一份可用的
    ///   默认配置，用户随后在详情里完善」。
    ///
    /// 默认实现只给 `id` / `name`（对无必填字段的插件即够）。有必填字段的插件
    /// 覆盖本方法，给出自己的最小合法配置——**否则该类型在 VDFS 侧无法新建**。
    fn new_entity_manifest(&self, id: &str, title: &str) -> serde_json::Value {
        serde_json::json!({ "id": id, "name": title })
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

/// 导航元数据（展示标签 + 顺序）—— 注册表是**单一真相源**。
///
/// 供不经过 [`EntityVdfsAdapter`] 的挂载点（session / setting 等自持 `VdfsProvider`
/// 的插件）复用，使 `.vdfs` 左栏的顺序与标签和实体注册表**恒等**；注册表调整顺序
/// 时各处导航自动跟随，无需改常量。未登记的 kind 返回 `None`。
///
/// [`EntityVdfsAdapter`]: crate::symbio_core::vdfs::EntityVdfsAdapter
pub fn nav_meta_of(kind: &str) -> Option<(&'static str, i32)> {
    provider_registry()
        .iter()
        .find(|p| p.kind == kind)
        .map(|p| (p.label, p.order))
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

/// 写入（创建或覆盖）一个**顶层实体**——实体机制与 VDFS 共用的唯一实现。
///
/// 职责链：`validate_manifest` 规范化 → 写盘 → `on_uploaded` 内存同步 →
/// 发布实体生命周期事件。
///
/// `entities/upload` 的 manifest 分支与 `VdfsProvider::write` 都走这里，
/// 两条链路因此行为完全一致（同一份校验、同一份写盘、同一个事件）。
pub async fn entity_write<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
    id: &str,
    manifest: &serde_json::Value,
) -> Result<EntityUploadResponse, PluginError> {
    let Some(category) = provider.category() else {
        // 无实体目录的实体（如 session / agent bundle）：协议槽位，走各自通道
        return Err(PluginError::NotImplemented);
    };
    let Some(manifest_file) = provider.manifest_file() else {
        return Err(PluginError::ValidationError(
            "该实体不支持表单上传（manifest）".to_string(),
        ));
    };

    let store = storage_service(ctx)?;
    let es = store.entity_store();
    let existed = es
        .entity_exists(category, id)
        .await
        .map_err(|e| PluginError::InternalError(format!("查询实体失败: {e}")))?;

    let normalized = provider.validate_manifest(ctx, id, manifest).await?;
    let content = serde_json::to_string_pretty(&normalized)?;
    es.write_entity(category, id, manifest_file, &content)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入实体失败: {e}")))?;

    provider.on_uploaded(ctx, id).await?;

    // 实体生命周期变更通知：前端据此即时同步清单（created 乐观插入 / updated 重拉）。
    // 事件总线不可用不应影响写入本身的结果。写入实体均为顶层（parent_id = None）。
    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        provider.kind(),
        id,
        if existed { "updated" } else { "created" },
        None,
        None,
    )
    .await;

    Ok(EntityUploadResponse {
        kind: provider.kind().to_string(),
        id: id.to_string(),
        created: !existed,
    })
}

/// 删除一个**顶层实体**——实体机制与 VDFS 共用的唯一实现。
///
/// 职责链：`delete_item` 落盘删除 → `on_deleted` 内存/缓存清理 → 发布生命周期事件。
pub async fn entity_delete<P: EntityProvider + ?Sized>(
    provider: &P,
    ctx: &Arc<dyn InvokeRequest>,
    id: &str,
) -> Result<EntityUploadResponse, PluginError> {
    provider.delete_item(ctx, id).await?;
    provider.on_deleted(ctx, id).await?;

    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        provider.kind(),
        id,
        "deleted",
        None,
        None,
    )
    .await;

    Ok(EntityUploadResponse {
        kind: provider.kind().to_string(),
        id: id.to_string(),
        created: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表顺序 = 导航顺序（会话 / 模型 / 智能体 / 技能 / MCP / 设置）
    #[test]
    fn registry_order_defines_nav_order() {
        let kinds: Vec<&str> = provider_registry().iter().map(|p| p.kind).collect();
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
        assert!(
            provider_registry()
                .windows(2)
                .all(|w| w[0].order <= w[1].order),
            "order 应单调不减"
        );
    }

    /// 导航元数据（标签 / 顺序）来自注册表——单一真相源
    #[test]
    fn nav_meta_comes_from_registry() {
        for p in provider_registry() {
            assert_eq!(nav_meta_of(p.kind), Some((p.label, p.order)));
        }
        assert_eq!(nav_meta_of("nope"), None, "未登记的 kind 返回 None");
    }
}
