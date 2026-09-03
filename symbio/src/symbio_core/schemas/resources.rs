//! 统一资源协议（五类资源：model / mcp / agent / skill / session 的对外契约）
//!
//! ## 设计原则
//!
//! - **机制统一、最小差异**：五类资源在服务器端都坐落于
//!   `~/.symbio/plugins/<category>/<id>/`（经 `EntityStore`），共享同一套
//!   `resources/*` 操作集（list / get / upload / delete / status）。
//! - **能力开关**：不同资源只在 `ResourceCapabilities` 上取值不同
//!   （zip 上传 / 独立表单 / 实时状态 / 可写 / 连接测试），前端据此驱动 UI，
//!   从而让"一份页面实例化五份"成为可能。
//! - 各插件在 `resources/*` 内**复用自身已有内部逻辑**，仅统一对外响应结构。
//!
//! ## 统一路径（各插件 route 分支）
//!
//! ```text
//! resources/list     — 列出全部资源（含能力开关 + 概要列表）
//! resources/get      — 读取单个资源详情
//! resources/upload   — 创建/更新（zip 解压 或 JSON manifest 表单）
//! resources/delete   — 删除
//! resources/status   — 读取单个资源实时/连接状态（可选能力）
//! ```

use serde::{Deserialize, Serialize};

// ==================== 资源类型常量 ====================

/// Model Provider
pub const RESOURCE_MODEL: &str = "model";
/// MCP Server
pub const RESOURCE_MCP: &str = "mcp";
/// Agent（智能体）
pub const RESOURCE_AGENT: &str = "agent";
/// Skill（技能）
pub const RESOURCE_SKILL: &str = "skill";
/// Session（会话）
pub const RESOURCE_SESSION: &str = "session";
/// Setting（设置分区）
pub const RESOURCE_SETTING: &str = "setting";

/// 与 [`crate::symbio_core::providers::storage::categories`] 一一对应的资源类型
pub const ALL_RESOURCE_TYPES: [&str; 5] = [
    RESOURCE_MODEL,
    RESOURCE_MCP,
    RESOURCE_AGENT,
    RESOURCE_SKILL,
    RESOURCE_SESSION,
];

// ==================== 统一路径常量 ====================

/// resources/list — 列出全部资源
pub const RESOURCES_LIST: &str = "resources/list";
/// resources/get — 读取单个资源详情
pub const RESOURCES_GET: &str = "resources/get";
/// resources/upload — 创建或更新资源
pub const RESOURCES_UPLOAD: &str = "resources/upload";
/// resources/delete — 删除资源
pub const RESOURCES_DELETE: &str = "resources/delete";
/// resources/status — 查询资源实时/连接状态
pub const RESOURCES_STATUS: &str = "resources/status";

// ==================== 能力开关 ====================

/// 资源能力开关 —— 决定该类型资源的统一页面启用哪些模块。
///
/// 前端可据此决定：走 zip 上传还是表单、是否需要状态轮询、能否删除等。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceCapabilities {
    /// 以上传 zip 为主（文件名即资源目录名）。`mcp` / `skill` / `agent` 为 true。
    pub zip_upload: bool,
    /// 是否有独立表单（`model` / `session` 先有，其余可后续扩展）。
    pub independent_form: bool,
    /// 列表项是否有实时状态（`session` 的 is_working、`mcp` 的连接状态等）。
    pub realtime_status: bool,
    /// 是否可写（可上传新增 / 删除）。只读资源（如某些 skill）为 false。
    pub mutable: bool,
    /// 是否支持"连接测试"（`model` / `mcp`）。
    pub test_connection: bool,
    /// 是否默认只读（当前轮次暂不可由 UI 增删）。
    pub read_only: bool,
}

impl ResourceCapabilities {
    /// model：表单为主，可测试、可写，无 zip
    pub const MODEL: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: false,
        mutable: true,
        test_connection: true,
        read_only: false,
    };

    /// mcp：zip 为主，实时连接状态，可测试、可写
    pub const MCP: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: true,
        mutable: true,
        test_connection: true,
        read_only: false,
    };

    /// skill：zip 为主，可写，无实时状态/无连接测试
    pub const SKILL: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: false,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// agent：zip 为主，可写，无实时状态
    pub const AGENT: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: false,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// session：表单为主，实时状态（is_working），可写
    pub const SESSION: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: true,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// setting：各分区有独立表单（前端按 config_type 注入 editor），
    /// 不可新建/删除（分区清单固定），保存由各 editor 自持通道完成
    pub const SETTING: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: false,
        mutable: false,
        test_connection: false,
        read_only: false,
    };
}

/// 默认能力表：`kind -> capabilities`
pub fn capabilities_for(kind: &str) -> ResourceCapabilities {
    match kind {
        RESOURCE_MODEL => ResourceCapabilities::MODEL,
        RESOURCE_MCP => ResourceCapabilities::MCP,
        RESOURCE_SKILL => ResourceCapabilities::SKILL,
        RESOURCE_AGENT => ResourceCapabilities::AGENT,
        RESOURCE_SESSION => ResourceCapabilities::SESSION,
        RESOURCE_SETTING => ResourceCapabilities::SETTING,
        _ => ResourceCapabilities {
            zip_upload: false,
            independent_form: false,
            realtime_status: false,
            mutable: false,
            test_connection: false,
            read_only: true,
        },
    }
}

// ==================== provider 注册信息 ====================

/// 资源类型（provider）注册信息 —— 宿主级单一真相源
///
/// 由核心层 [`crate::symbio_core::resources::provider_registry`] 静态注册，
/// 经宿主 `resources/providers` 端点下发，前端据此动态生成左侧导航栏与
/// 统一资源页的类型集合（替代硬编码类型清单）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// 资源类型（kind），如 `model` / `mcp` / `session`
    pub kind: String,
    /// 提供方显示名，用于路径 `[provider]/[id].[kind]`
    pub provider_name: String,
    /// 资源操作前缀（前端拼接 `${prefix}/resources/<op>`）
    pub prefix: String,
    pub capabilities: ResourceCapabilities,
    /// 展示顺序（前端导航/类型选择排序）
    pub order: i32,
    /// 展示标签（如 `Model` / `Session`）
    pub label: String,
    /// 是否支持在资源管理器内创建/删除（`category` 存在且 dispatch 实现 upload/delete）
    pub supports_upload: bool,
    /// 列表简洁模式：仅显示类型图标 + 标题（如设置分区，无描述/路径标签）
    #[serde(default)]
    pub compact_list: bool,
}

/// `resources/providers` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}

// ==================== 统一列表项 ====================

/// 统一资源概要（列表项）
///
/// `status` 取值建议：`active` / `disabled` / `working` / `error` / `unknown`。
/// `extra` 展开存放类型特有字段（如 model 的 provider/model、session 的
/// message_count 等），前端按需读取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSummary {
    /// 资源类型标识（`model` / `mcp` / `agent` / `skill` / `session`）
    pub kind: String,
    /// 提供方（插件）显示名，用于前端资源路径 `[provider]/[id].[kind]` 展示；
    /// 由 dispatch 统一回填（默认与 kind 相同），插件无需关心
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 显示名
    pub name: String,
    /// 唯一 id（即服务器端目录名）
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 一行摘要（如 skill 的 body 开头 / agent 描述）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 最近更新时间（秒时间戳）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// 状态
    pub status: String,
    /// 状态补充说明（如连接失败原因、等待审批）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    /// 类型特有扩展字段
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl ResourceSummary {
    pub fn new(kind: &str, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            id: id.into(),
            name: name.into(),
            provider: None,
            description: None,
            summary: None,
            updated_at: None,
            status: "active".to_string(),
            status_detail: None,
            extra: serde_json::Value::Object(Default::default()),
        }
    }
}

// ==================== 请求 / 响应 ====================

/// `resources/list` 响应：能力开关 + 资源概要列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesListResponse {
    pub kind: String,
    pub capabilities: ResourceCapabilities,
    pub items: Vec<ResourceSummary>,
}

/// `resources/upload` 请求
///
/// 上传方式二选一：
/// - `zip_b64`：zip 字节的 base64（mcp / skill / agent）；`name` 即目标目录名
/// - `manifest`：JSON 表单体（model / session）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUploadRequest {
    pub kind: String,
    /// 目标资源名 / 目录名。zip 上传必填。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// zip 字节（base64）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_b64: Option<String>,
    /// 表单体（JSON），供 independent_form 资源使用
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    /// 已存在时是否覆盖（默认 true）
    #[serde(default = "default_replace")]
    pub replace: bool,
}

const fn default_replace() -> bool {
    true
}

/// `resources/upload` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUploadResponse {
    pub kind: String,
    pub id: String,
    pub created: bool,
}

/// `resources/get` 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGetRequest {
    pub kind: String,
    pub id: String,
}

/// `resources/delete` 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDeleteRequest {
    pub kind: String,
    pub id: String,
}

/// `resources/status` 请求
///
/// 仅发起状态查询的资源标识；实现可按类型复用内部状态源。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatusRequest {
    pub kind: String,
    pub id: String,
}

/// `resources/status` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatusResponse {
    pub kind: String,
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
}