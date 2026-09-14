//! 实体机制的**宿主方言**（S11 之后）
//!
//! `entities/*` **调用协议已下线**，对外地址只剩 VDFS（`.vdfs/…`）。本模块
//! 因此不再是「对外契约」，而是 VDFS `ext = form` 这一**呈现形态的宿主方言**：
//!
//! - [`DetailDefinition`] 及其附属形状（`DetailSection` / `DetailField` /
//!   `DetailOption` / `DetailBadge` / `DetailAction` / 预设 …）：后端下发、
//!   前端通用渲染器（DetailForm）据此渲染详情，**零页面开发**；
//! - [`EntitySummary`]：provider 之间的交换形状（列表项 / 详情数据源）；
//! - [`EntityUploadResponse`] / [`EntityStatusResponse`]：写盘与状态钩子的
//!   返回形状（VDFS 侧的写 / 动作结果由此派生）。
//!
//! 能力判定不在本模块：可写性来自 **VDFS 访问位**，可新建 / 可导入来自
//! `provider_registry()` 的 `supports_upload` / `supports_import`，可测试
//! 与否来自 `DetailDefinition.actions` 里声明的动作。

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

// ==================== 统一列表项 ====================

/// 实体概要（列表项）
///
/// `status` 取值建议：`active` / `disabled` / `working` / `error` / `unknown`。
/// `extra` 展开存放类型特有字段（如 model 的 provider/model、session 的
/// message_count 等），前端按需读取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySummary {
    /// 实体类型标识（`model` / `mcp` / `agent` / `skill` / `session`）
    pub kind: String,
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

/// 实体写入（`vdfs/write` / 整包导入）响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityUploadResponse {
    pub kind: String,
    pub id: String,
    pub created: bool,
}

/// 整包导出结果（VDFS 动作 `export` 的 `data` 载荷）
///
/// 与「新建类型 `zip`」的导入互为逆向：导入把 zip 字节展开成实体目录，
/// 导出把实体目录打包成 zip 字节（base64，走 VDFS 的二进制通道）。
///
/// 字段名与 [`VdfsContent::b64`] 同构——前端据此把它当**文件载荷**处理
/// （有 `filename` + `b64` 就下载），不认识「导出」这个动作本身。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityExport {
    /// 被导出的实体 id
    pub id: String,
    /// 建议文件名（`<id>.zip`）
    pub filename: String,
    /// 打包字节（base64）
    pub b64: String,
}

/// 实体状态（`vdfs/action { action: "test" }`）响应
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

/// 条件谓词（徽标/动作显隐/字段显隐）。`all` 存在时为 AND 组合，其余字段忽略。
///
/// 语义是**中性的「条件成立」**，由使用方决定成立意味着什么：
///
/// | 使用处 | 成立 ⇒ | 缺省（无该字段） |
/// |---|---|---|
/// | `when`（动作 / 徽标） | 显示 | 显示 |
/// | `disabled_when`（动作） | **禁用** | 不禁用 |
/// | `visible_when`（字段） | 显示 | 显示 |
///
/// 因此「缺省」一律按**成立**处理（渲染器侧对空条件求值为 `true`）；只有
/// `disabled_when` 需要渲染器额外判空——否则「无条件」会被解释成「禁用」。
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
    /// 显隐条件：**成立才显示**（缺省 = 显示）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<DetailCondition>,
    /// 禁用条件：**成立才禁用**（缺省 = 不禁用）。
    /// 例如 `{key: "is_existing", equals: false}` = 新建态禁用「删除」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<DetailCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_label: Option<String>,
}

/// 详情页定义。`binding` ∈ upload（实体：预填 item.config，保存走
/// manifest 写入）| config（配置分区：经 `load_path`/
/// `save_path` 读写，如 `config/get` / `config/set`）| info（只读概览：
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
