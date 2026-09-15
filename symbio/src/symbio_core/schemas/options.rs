//! 级联选项机制协议（cascading options）
//!
//! ## 目标
//!
//! 会话页输入区下方的「选项行」不再由前端写死，而由**后端下发**：
//! 插件经统一收集机制（`OptionVisitor` + `Plugin::traverse`）贡献选项节点，
//! 会话插件作为**选项宿主**在 `options/list` 上一次下发根选项列表。
//! 前端只实现一套渲染机制（`ChatOptionBar`），不含任何具体业务选项。
//!
//! ## 三类选项（[`OptionType`]）
//!
//! | 类型 | 语义 | 交互 |
//! |---|---|---|
//! | `invoke` | 调用特定后端服务 | 点击 → 合并参数 → `callPlugin(action.endpoint, payload)` |
//! | `sub` | 子选项列表（级联） | 展开 → 子项；子项自身仍是选项节点（通常为 `invoke`） |
//! | `form` | 自动化表单 | 打开表单（复用 `DetailDefinition` / `DetailForm`），保存 → 调用 `action.endpoint` |
//!
//! `sub` 的层级是**任意深度**的：子项可再是 `sub`（级联），子项内联下发
//! （`children`）或经 `options/list` 的 `parent` 参数懒加载。
//!
//! ## 协议端点（由选项宿主 = session 插件提供）
//!
//! ```text
//! worker/session/options/list   → OptionsResponse（parent 缺省 = 根层；非空 = 该节点的子项）
//! ```
//!
//! 单个端点是刻意的：根层与子层只是 `parent` 参数的有无，与 VDFS 的
//! 树懒加载（`vdfs/list` 的 `parent`）同构，避免为同一机制造第二个通道。
//!
//! ## 与详情表单方言的统一（`docs/design/vdfs.md` §7）
//!
//! `form` 类型直接复用详情表单方言 [`DetailDefinition`]（字段/分区/条件显隐/
//! 预设联动的表达能力见 `crate::symbio_core::schemas::detail`）：与 VDFS 节点
//! `ext = form` 的详情页完全一致，前端复用唯一渲染器 `DetailForm`
//! （`binding = "option"`：预填自节点 `data`，保存回 `action`）。
//!
//! ## 状态落库
//!
//! 状态型选项（当前选中值）由贡献方在节点上带出（`value` / `value_label`）；
//! 用户选择后统一经 [`SESSION_STATE_ENDPOINT`]（`worker/session/update`）
//! 合并写入 `session.metadata`——后端各解析链（orchestrator / tool_executor）
//! 已按 metadata 回退取值，因此会话参数无需前端参与。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `options/list` —— 选项列表（根层 / 子层由 `parent` 参数区分）
pub const OPTIONS_LIST: &str = "options/list";

/// 选项机制的标准「状态落库」服务路径。
///
/// 状态型选项（智能体 / 模型 / 模式 / 风险等级 / 工作目录 / 心跳配置…）的
/// 选择统一经此端点合并写入 `session.metadata`：一处落库、后端单一真相源，
/// 前端不持有任何业务字段名。
pub const SESSION_STATE_ENDPOINT: &str = "worker/session/update";

/// 状态取值约定（与 VDFS 节点 `status` 同一套语义，见 `docs/design/vdfs.md` §3.2）
pub const OPTION_STATUS_ACTIVE: &str = "active";
pub const OPTION_STATUS_WORKING: &str = "working";
pub const OPTION_STATUS_DISABLED: &str = "disabled";
pub const OPTION_STATUS_ERROR: &str = "error";
pub const OPTION_STATUS_UNKNOWN: &str = "unknown";

/// 机制原生取值原语（闭集）——前端实现的通用取值能力，不含任何业务语义。
///
/// 后端无法唤起原生对话框，故 `invoke` 动作可声明一个原生取值原语：
/// 前端先取值、写入 `action.bind` 指定的参数路径，再调用 `action.endpoint`。
pub const OPTION_PICK_DIRECTORY: &str = "directory";
/// 原生文件选择
pub const OPTION_PICK_FILE: &str = "file";

fn default_true() -> bool {
    true
}

/// 选项栏（会话输入区下方）显示策略 —— 机制级、由后端声明，前端零写死。
///
/// 仅 [`OptionDisplay::show_label`] 一个开关：是否在选项栏展示类别标签
/// （`label`）。缺省 `true`（现行「图标 + 类别标签 + 当前值」双段渲染）；
/// `false` = 仅「图标 + 当前值」，类别名整体移入悬停提示——用于压缩横向
/// 空间。前端只读取此字段，不自行决定显示策略。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionDisplay {
    /// 是否在选项栏显示类别标签。缺省 true。
    #[serde(default = "default_true")]
    pub show_label: bool,
}

/// 选项节点类型
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionType {
    /// 调用特定后端服务（`action.endpoint`）
    #[default]
    Invoke,
    /// 子选项列表（级联；`children` 内联或经 `parent` 懒加载）
    Sub,
    /// 自动化表单（复用 `DetailDefinition` / `DetailForm`）
    Form,
}

/// 选项动作 —— `invoke` / `form` 两类选项的执行规格。
///
/// - `endpoint`：后端服务路径（前端 `callPlugin` 调用），如
///   [`SESSION_STATE_ENDPOINT`]；
/// - `payload`：固定参数（与动态参数合并后下发）；
/// - `pick` + `bind`：机制原生取值原语 + 其写入的参数路径
///   （点路径，如 `metadata.workdir`）。`pick` 与 `bind` 成对出现。
///
/// `form` 保存时，表单字段值写入 `bind` 指定的路径（缺省 = 平铺进 payload）；
/// `pick` 时，原生取回的值写入 `bind` 指定的路径。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionAction {
    /// 后端服务路径（空 = 纯原生动作，仅取值不外呼）
    pub endpoint: String,
    /// 固定参数（动态参数合并其上）
    pub payload: Value,
    /// 机制原生取值原语（闭集）：`directory` / `file`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    /// 动态参数的写入路径（点路径；`pick` 与 `form` 保存共用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
}

/// 选项节点 —— 机制下发的唯一形态单位。
///
/// 显示信息：`label` / `icon` / `description` / `value` / `value_label`；
/// 状态信息：`status` / `status_detail` / `enabled`；
/// 类型与类型专属数据：`option_type` + `action` / `children` / `form`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionNode {
    /// 节点 id（会话作用域内唯一；子项 id 建议带父前缀，便于定位）
    pub id: String,
    /// 显示名（如「工作目录」「智能体」）
    pub label: String,
    /// 图标名（前端 UI 资产映射；缺省按节点语义回落）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 语义说明（悬浮提示 / 子项说明）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 类型
    pub option_type: OptionType,
    /// 展示顺序（同一宿主下升序；跨插件贡献时必需，保证稳定序）
    pub order: i32,
    /// 状态（[`OPTION_STATUS_ACTIVE`] 等）
    pub status: String,
    /// 状态补充说明
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    /// 当前选中值（状态型选项；子项 `value` 与父节点 `value` 对应）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// 当前选中值的展示文本（缺省 = `value`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_label: Option<String>,
    /// 是否可选（false = 只读展示）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// invoke / form：执行规格
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<OptionAction>,
    /// form：表单定义（与资源详情表单同一套 schema）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<super::detail::DetailDefinition>,
    /// form：表单初始数据（字段名 → 值；前端 DetailForm 预填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// sub：子选项（内联下发；空 = 懒加载，经 `parent` 请求）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<OptionNode>,
    /// 选项栏显示策略（机制级；后端声明，前端据此渲染，不写死）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<OptionDisplay>,
}

impl Default for OptionNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            icon: None,
            description: None,
            option_type: OptionType::Invoke,
            order: 0,
            status: OPTION_STATUS_ACTIVE.to_string(),
            status_detail: None,
            value: None,
            value_label: None,
            enabled: true,
            action: None,
            form: None,
            data: None,
            children: Vec::new(),
            display: None,
        }
    }
}

impl OptionNode {
    /// 构造一个可用状态的基础节点
    pub fn new(id: impl Into<String>, label: impl Into<String>, option_type: OptionType) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            option_type,
            ..Default::default()
        }
    }

    /// 便捷构造：`invoke` 节点
    pub fn invoke(id: impl Into<String>, label: impl Into<String>, action: OptionAction) -> Self {
        Self {
            action: Some(action),
            ..Self::new(id, label, OptionType::Invoke)
        }
    }

    /// 便捷构造：`sub` 节点
    pub fn sub(id: impl Into<String>, label: impl Into<String>, children: Vec<OptionNode>) -> Self {
        Self {
            children,
            ..Self::new(id, label, OptionType::Sub)
        }
    }

    /// 链式：设置图标
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// 链式：设置展示顺序
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// 链式：设置当前选中值（展示文本缺省 = 值本身）
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        let v = value.into();
        self.value_label = Some(v.clone());
        self.value = Some(v);
        self
    }

    /// 链式：设置当前选中值 + 独立展示文本
    pub fn with_value_label(mut self, value: impl Into<String>, label: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self.value_label = Some(label.into());
        self
    }

    /// 链式：设置状态
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    /// 链式：设置可选性（false = 只读展示，前端不响应点击）
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 链式：设置说明
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 链式：设置选项栏显示策略（机制级）。`show_label = false` 时仅显示
    /// 「图标 + 当前值」，类别标签移入悬停提示（用于压缩横向空间）。
    pub fn with_display(mut self, show_label: bool) -> Self {
        self.display = Some(OptionDisplay { show_label });
        self
    }
}

impl OptionAction {
    /// 「会话状态落库」动作（固定值）：选中即把 `metadata[key] = value`
    /// 合并写入会话。状态型选项目前全部经此落库，后端各解析链按
    /// `session.metadata` 回退取值（见 [`SESSION_STATE_ENDPOINT`]）。
    pub fn session_state_set(key: &str, value: Value) -> Self {
        Self {
            endpoint: SESSION_STATE_ENDPOINT.to_string(),
            payload: serde_json::json!({ "metadata": { key: value } }),
            pick: None,
            bind: None,
        }
    }

    /// 「会话状态落库」动作（点路径）：动态值（原生取值 / 表单参数）
    /// 写入 `bind` 指定的点路径后再调用（如 `metadata.workdir`）。
    pub fn session_state_bind(bind: &str) -> Self {
        Self {
            endpoint: SESSION_STATE_ENDPOINT.to_string(),
            payload: serde_json::json!({ "metadata": {} }),
            pick: None,
            bind: Some(bind.to_string()),
        }
    }
}

impl OptionNode {
    /// 「会话状态」选项子项：选中即把 `metadata[key] = value` 落库。
    ///
    /// `value` 仅为该子项自身的取值（前端按 `child.value == parent.value`
    /// 判定选中态）；`label` 才是展示文本。
    pub fn session_state(
        id: impl Into<String>,
        label: impl Into<String>,
        key: &str,
        value: Value,
    ) -> Self {
        let display = match &value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        Self::invoke(id, label, OptionAction::session_state_set(key, value)).with_value(display)
    }
}

/// `options/list` 请求。
///
/// - `session_id`：会话作用域（选项宿主据此把会话状态（agent_id /
///   provider_id / mode / risk_level / workdir）注入收集上下文，供各贡献
///   插件回填「当前选中值」）；
/// - `parent`：父节点 id；缺省 = 根层（会话输入区的根选项列表）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// `options/list` 响应（根层或某一父节点的子项）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionsResponse {
    pub nodes: Vec<OptionNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_serde_roundtrip_and_defaults() {
        let node = OptionNode::invoke(
            "workdir",
            "工作目录",
            OptionAction {
                endpoint: SESSION_STATE_ENDPOINT.to_string(),
                payload: serde_json::json!({"metadata": {}}),
                pick: Some(OPTION_PICK_DIRECTORY.to_string()),
                bind: Some("metadata.workdir".to_string()),
            },
        )
        .with_icon("folder")
        .with_order(10)
        .with_value("D:/x");

        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["option_type"], serde_json::json!("invoke"));
        assert_eq!(v["action"]["pick"], serde_json::json!("directory"));
        assert_eq!(v["status"], serde_json::json!("active"));
        assert_eq!(v["enabled"], serde_json::json!(true));
        // 空字段不输出（children / form / data 缺省省略）
        assert!(v.get("children").is_none());
        assert!(v.get("form").is_none());

        let back: OptionNode = serde_json::from_value(v).unwrap();
        assert_eq!(back, node);
    }

    #[test]
    fn node_absent_enabled_defaults_true() {
        // 旧/精简 JSON（无 enabled / status / option_type）反序列化：
        // enabled 缺省 true、option_type 缺省 invoke、status 缺省 active
        let node: OptionNode = serde_json::from_value(serde_json::json!({
            "id": "x", "label": "X"
        }))
        .unwrap();
        assert!(node.enabled);
        assert_eq!(node.option_type, OptionType::Invoke);
        assert_eq!(node.status, OPTION_STATUS_ACTIVE);
    }

    #[test]
    fn sub_node_carries_children() {
        let child = OptionNode::invoke(
            "mode:auto",
            "自动",
            OptionAction {
                endpoint: SESSION_STATE_ENDPOINT.to_string(),
                payload: serde_json::json!({"metadata": {"mode": "auto"}}),
                ..Default::default()
            },
        );
        let parent = OptionNode::sub("mode", "运行模式", vec![child]).with_value("auto");
        let v = serde_json::to_value(&parent).unwrap();
        assert_eq!(v["option_type"], serde_json::json!("sub"));
        assert_eq!(v["children"].as_array().unwrap().len(), 1);
        assert_eq!(v["value"], serde_json::json!("auto"));
    }
}
