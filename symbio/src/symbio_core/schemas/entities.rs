//! 统一实体协议（五类实体：model / mcp / agent / skill / session 的对外契约）
//!
//! ## 设计原则
//!
//! - **机制统一、最小差异**：五类实体在服务器端都坐落于
//!   `~/.symbio/plugins/<category>/<id>/`（经 `EntityStore`），共享同一套
//!   `entities/*` 操作集（list / get / upload / delete / status）。
//! - **能力开关**：不同实体只在 `EntityCapabilities` 上取值不同
//!   （zip 上传 / 独立表单 / 实时状态 / 可写 / 连接测试），前端据此驱动 UI，
//!   从而让"一份页面实例化五份"成为可能。
//! - 各插件在 `entities/*` 内**复用自身已有内部逻辑**，仅统一对外响应结构。
//!
//! ## 统一路径（各插件 route 分支）
//!
//! ```text
//! entities/list     — 列出全部实体（含能力开关 + 概要列表）
//! entities/get      — 读取单个实体详情
//! entities/upload   — 创建/更新（zip 解压 或 JSON manifest 表单）
//! entities/delete   — 删除
//! entities/status   — 读取单个实体实时/连接状态（可选能力）
//! ```

use serde::{Deserialize, Serialize};

// ==================== 实体类型常量 ====================

/// Model Provider
pub const ENTITY_MODEL: &str = "model";
/// MCP Server
pub const ENTITY_MCP: &str = "mcp";
/// Agent（智能体）
pub const ENTITY_AGENT: &str = "agent";
/// Skill（技能）
pub const ENTITY_SKILL: &str = "skill";
/// Session（会话）
pub const ENTITY_SESSION: &str = "session";
/// Setting（设置分区）
pub const ENTITY_SETTING: &str = "setting";

// ==================== 能力开关 ====================

/// 实体能力开关 —— 决定该类型实体的统一页面启用哪些模块。
///
/// 前端可据此决定：走 zip 上传还是表单、是否需要状态轮询、能否删除等。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityCapabilities {
    /// 以上传 zip 为主（文件名即实体目录名）。`mcp` / `skill` / `agent` 为 true。
    pub zip_upload: bool,
    /// 是否有独立表单（`model` / `session` 先有，其余可后续扩展）。
    pub independent_form: bool,
    /// 列表项是否有实时状态（`session` 的 is_working、`mcp` 的连接状态等）。
    pub realtime_status: bool,
    /// 列表头部是否提供「刷新」动作。清单内容可能静默变化（外部修改、
    /// 事件未覆盖）的类型为 true；清单由生命周期事件通道自持同步
    /// （`session`）或固定不变（`setting`）的类型为 false——前端据此
    /// 决定列表头是否渲染刷新按钮，不得自行判断（§3.4）。
    pub refreshable: bool,
    /// 是否可写（可上传新增 / 删除）。只读实体（如某些 skill）为 false。
    pub mutable: bool,
    /// 是否支持"连接测试"（`model` / `mcp`）。
    pub test_connection: bool,
    /// 是否默认只读（当前轮次暂不可由 UI 增删）。
    pub read_only: bool,
}

impl EntityCapabilities {
    /// model：表单为主，可测试、可写，无 zip；清单可能被外部修改，可刷新
    pub const MODEL: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: false,
        refreshable: true,
        mutable: true,
        test_connection: true,
        read_only: false,
    };

    /// mcp：zip 为主，实时连接状态，可测试、可写；清单可能被外部修改，可刷新
    pub const MCP: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: true,
        refreshable: true,
        mutable: true,
        test_connection: true,
        read_only: false,
    };

    /// skill：zip 为主，可写，无实时状态/无连接测试；清单可能被外部修改，可刷新
    pub const SKILL: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: false,
        refreshable: true,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// agent：zip 为主，可写，无实时状态；清单可能被外部修改，可刷新
    pub const AGENT: Self = Self {
        zip_upload: true,
        independent_form: false,
        realtime_status: false,
        refreshable: true,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// session：表单为主，实时状态（is_working），可写。
    /// 清单由实体生命周期事件通道自持同步（created/updated/deleted 即时收敛），
    /// 列表头不提供「刷新」（refreshable=false）。
    pub const SESSION: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: true,
        refreshable: false,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// setting：各分区有独立表单（前端按 config_type 注入 editor）。
    /// 分区清单固定——列表头无「新建/刷新」（refreshable=false），
    /// 保存由各 editor 自持通道完成
    pub const SETTING: Self = Self {
        zip_upload: false,
        independent_form: true,
        realtime_status: false,
        refreshable: false,
        mutable: false,
        test_connection: false,
        read_only: false,
    };

    /// 容器子实体（文件级）：可写可删，无 zip / 表单 / 实时状态 / 连接测试。
    /// agent bundle 内部的 prompt / skill / mcp 等单文件实体取此形态；
    /// 清单可能被外部修改，可刷新。
    pub const BUNDLE_FILE: Self = Self {
        zip_upload: false,
        independent_form: false,
        realtime_status: false,
        refreshable: true,
        mutable: true,
        test_connection: false,
        read_only: false,
    };

    /// 容器子实体（系统管理型）：可删不可创建（子会话由父会话派生，
    /// 非用户新建——对应容器声明 `path_hint` 为空）；清单随会话活动变化，
    /// 可刷新。
    pub const SUB_SESSION: Self = Self {
        zip_upload: false,
        independent_form: false,
        realtime_status: false,
        refreshable: true,
        mutable: true,
        test_connection: false,
        read_only: false,
    };
}

/// 默认能力表：`kind -> capabilities`
pub fn capabilities_for(kind: &str) -> EntityCapabilities {
    match kind {
        ENTITY_MODEL => EntityCapabilities::MODEL,
        ENTITY_MCP => EntityCapabilities::MCP,
        ENTITY_SKILL => EntityCapabilities::SKILL,
        ENTITY_AGENT => EntityCapabilities::AGENT,
        ENTITY_SESSION => EntityCapabilities::SESSION,
        ENTITY_SETTING => EntityCapabilities::SETTING,
        _ => EntityCapabilities {
            zip_upload: false,
            independent_form: false,
            realtime_status: false,
            // 未登记类型：刷新幂等无害，保守保留入口
            refreshable: true,
            mutable: false,
            test_connection: false,
            read_only: true,
        },
    }
}

// ==================== 统一列表项 ====================

/// 统一实体概要（列表项）
///
/// `status` 取值建议：`active` / `disabled` / `working` / `error` / `unknown`。
/// `extra` 展开存放类型特有字段（如 model 的 provider/model、session 的
/// message_count 等），前端按需读取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySummary {
    /// 实体类型标识（`model` / `mcp` / `agent` / `skill` / `session`）
    pub kind: String,
    /// 提供方（插件）显示名，用于前端实体路径 `[provider]/[id].[kind]` 展示；
    /// 由实体机制统一回填（默认与 kind 相同），插件无需关心
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
    /// 父节点 id（树视图专用：`view = "tree"` 的子类别条目以容器内相对路径
    /// 为 id、父路径为 parent 构成层级；根层条目缺省。列表视图不用此字段）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// 可展开提示（树视图专用：true = 该节点可懒加载下一层；缺省按可展开
    /// 处理，展开请求返回空则收敛为叶子。列表视图不用此字段）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expandable: Option<bool>,
    /// 类型特有扩展字段
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl EntitySummary {
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
            parent: None,
            expandable: None,
            extra: serde_json::Value::Object(Default::default()),
        }
    }
}

// ==================== 请求 / 响应 ====================

/// `entities/list` 响应：能力开关 + 实体概要列表
///
/// 容器语义（请求携带 `container`）时，`items` 为容器条目内部的子实体
/// （条目 `kind` 字段区分子类型），`container` 回显容器 id。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitiesListResponse {
    pub kind: String,
    pub capabilities: EntityCapabilities,
    pub items: Vec<EntitySummary>,
    /// 容器作用域（容器语义时下发；顶层列表为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// `entities/list` 请求（可选容器作用域）
///
/// 不带 `container`：列出 provider 顶层实体（现有语义，向后兼容）；
/// 带 `container`：列出容器条目内部的子实体（如某 agent bundle 的 prompts/skills/mcps）。
/// 树视图子类别（`view = "tree"`）带 `parent`：返回该父路径的下一层子节点
/// （懒加载，`parent` 缺省 = 根层）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntitiesListRequest {
    /// 容器条目 id（如 agent bundle id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// 子实体类型过滤（缺省返回全部子类型，条目自带 kind 供前端分类）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_kind: Option<String>,
    /// 树视图父路径（容器内相对路径；缺省 = 根层；仅 `view = "tree"` 子类别使用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// `entities/upload` 请求
///
/// 上传方式二选一：
/// - `zip_b64`：zip 字节的 base64（mcp / skill / agent）；`name` 即目标目录名
/// - `manifest`：JSON 表单体（model / session；容器语义时取 `manifest.content`）
///
/// 带 `container` 时为容器语义：`name` 即容器内相对路径，`manifest.content`
/// 即文件内容（创建/覆盖）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityUploadRequest {
    pub kind: String,
    /// 目标实体名 / 目录名。zip 上传必填；容器语义下为容器内相对路径。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// zip 字节（base64）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_b64: Option<String>,
    /// 表单体（JSON），供 independent_form 实体使用；容器语义下取 `content` 字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    /// 已存在时是否覆盖（默认 true）
    #[serde(default = "default_replace")]
    pub replace: bool,
    /// 容器作用域：实体位于该容器条目内部（如 agent bundle id）。缺省 = 顶层实体
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

const fn default_replace() -> bool {
    true
}

/// `entities/upload` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityUploadResponse {
    pub kind: String,
    pub id: String,
    pub created: bool,
}

/// `entities/get` 请求
///
/// 带 `container` 时为容器语义：读取容器条目内部的子实体，`id` 为容器内相对路径，
/// 内容随 `EntitySummary.extra.content` 返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityGetRequest {
    pub kind: String,
    pub id: String,
    /// 容器作用域（如 agent bundle id）。缺省 = 顶层实体
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// `entities/delete` 请求（带 `container` 时为容器语义，`id` 为容器内相对路径）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDeleteRequest {
    pub kind: String,
    pub id: String,
    /// 容器作用域。缺省 = 顶层实体
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

/// `entities/status` 请求
///
/// 仅发起状态查询的实体标识；实现可按类型复用内部状态源。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityStatusRequest {
    pub kind: String,
    pub id: String,
}

/// `entities/status` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityStatusResponse {
    pub kind: String,
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
}

// ==================== 详情页定义（definition-driven detail） ====================
//
// 交互不复杂的详情页由后端下发**定义**、前端通用渲染器（DetailForm）动态生成，
// 前端零页面开发。定义能力：预设联动填充 / 动态候选（datalist/select）/
// 密码显隐 / 数字范围 / 折叠分区 / 条件徽标与动作 / id·name 派生回落链。
// 只读概览型详情（如 agent bundle 概览）由 `info` 绑定表达；
// 复杂详情（会话聊天工作区、appearance 即时生效型、about 信息展示型）
// 仍走注册 editor，不适用本定义。

/// 条件谓词（徽标/动作显隐）。`all` 存在时为 AND 组合，其余字段忽略。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailCondition {
    /// 求值键：表单字段名，或特殊键 `is_existing` / `is_default` / `cap.<name>`
    pub key: String,
    pub equals: Option<serde_json::Value>,
    pub not_equals: Option<serde_json::Value>,
    pub truthy: Option<bool>,
    /// AND 组合（嵌套条件全真才真）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<DetailCondition>,
}

/// select 选项 / 值-标签对
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DetailOption {
    pub value: String,
    pub label: String,
}

/// 表单字段定义。`widget` ∈ text | password | number | select | textarea |
/// toggle | datalist | list | map | static；`options`/`suggestions` 为静态候选，
/// `*_from_preset` 为真时候选来自当前预设的 `options[key]`（如 provider 预设注入模型列表）。
///
/// 结构化 widget 的表单模型约定（渲染器与 `validate_manifest` 两侧一致）：
/// - `list`：字符串数组，编辑态每行一项；
/// - `map`：字符串键值对，编辑态每行 `KEY=VALUE`；
/// - `static`：只读展示（info 绑定），值来自 `item.config`/`extra`，
///   `options` 可作值→标签映射。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailField {
    /// 绑定到表单模型的字段名
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub required: bool,
    pub widget: String,
    /// 条件显隐（不满足时整行不渲染；求值同徽标/动作条件）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<DetailCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// textarea 行数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<DetailOption>,
    /// datalist 静态建议
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    /// 候选来自预设注入（select 选项 / datalist 建议）
    pub options_from_preset: bool,
    pub suggestions_from_preset: bool,
    /// 整行布局（textarea 等宽控件）
    pub full_width: bool,
    /// 新建态缺省值
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// 分区（可折叠；`collapsed` = 默认折叠，如「高级设置」）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub collapsed: bool,
    pub fields: Vec<DetailField>,
}

/// 预设项：选中后按 `set` 填充字段值（策略见 [`DetailPresetSpec::fill`]），
/// 并把 `options`（字段名 → 候选列表）注入对应字段的动态候选。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailPreset {
    pub value: String,
    pub label: String,
    pub set: std::collections::BTreeMap<String, serde_json::Value>,
    /// 总是覆盖（不参与 fill 策略，如协议校正）
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub set_always: std::collections::BTreeMap<String, serde_json::Value>,
    pub options: std::collections::BTreeMap<String, Vec<String>>,
}

/// 预设联动规格：`field` 为触发预设的 select 字段；`fill` ∈ if_empty | always
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailPresetSpec {
    pub field: String,
    pub fill: String,
    pub presets: Vec<DetailPreset>,
}

/// 标题区徽标（如「默认」「已停用」），按条件显隐
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailBadge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<DetailCondition>,
    pub label: String,
    /// default | disabled | accent
    pub style: String,
}

/// 动作按钮。`id` ∈ save | test | delete | set-default | open-container
/// （机制语义动作，前端接统一通道；`open-container` 经 `payload.kind`
/// 路由推入容器实体页）或自定义（预留）；`payload` 合并进 save 负载
/// （如 `skip_validation`）；`busy_label` 为进行中文案。
///
/// `icon`：图标名（可选）。语义动作 id 自带默认图标映射（前端纯 UI 资产），
/// 仅当同一动作需要区分形态（如同为 save 的「跳过校验保存」）或自定义
/// 动作需要图标时才显式指定；未知图标名回落为文字按钮。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailAction {
    pub id: String,
    pub label: String,
    /// primary | secondary | danger | icon
    pub style: String,
    /// 图标名（缺省 = 按 id 的默认图标映射；无映射 → 文字按钮）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<DetailCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<DetailCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_label: Option<String>,
}

/// 详情页定义。`binding` ∈ upload（实体实体：预填 item.config，保存走
/// `entities/upload` manifest）| config（配置分区：经 `load_path`/
/// `save_path` 读写，如 `local/config get|set`）| info（只读概览：
/// 无保存，字段取值来自 item.config/extra，配 `static` widget 展示）。
/// 派生链均为「首个非空」：
/// `title_from` 生成标题，`name_from` 保存时补名称，`id_from` 新建时
/// 派生 slug id（前端去重 `-2` 递增，后端 `validate_manifest` 兜底）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailDefinition {
    pub binding: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub title_from: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_fallback: Option<String>,
    /// 副标题派生链（首个非空字段依次展示，如 provider 标签 / model）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subtitle_from: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub name_from: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub id_from: Vec<String>,
    pub sections: Vec<DetailSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presets: Option<DetailPresetSpec>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<DetailBadge>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<DetailAction>,
}

/// `entities/detail` 请求（`id` 为空 = 请求「新建态」定义）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetailDefinitionRequest {
    pub kind: String,
    pub id: String,
}

/// `entities/detail` 响应（`definition = None` 表示无定义）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetailDefinitionResponse {
    pub definition: Option<DetailDefinition>,
}
