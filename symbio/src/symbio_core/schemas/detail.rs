//! 详情表单的**宿主方言**（definition-driven detail）
//!
//! 这里讲的**不是 VDFS 机制**，而是 VDFS 之上的一种**呈现形态**：后端下发一份
//! [`DetailDefinition`]，前端通用渲染器（DetailForm）据此渲染详情与表单，
//! **零页面开发**。VDFS 侧只提供节点、访问位与读写通道，字段布局、预设联动、
//! 条件显隐全是宿主方言。
//!
//! 与 VDFS 的接缝只有两处，且都由 provider 自己声明，不存在机制侧的猜测：
//!
//! - 节点 `ext = form` → 前端用通用渲染器打开本模块下发的定义；
//! - 提交值的校验 → [`DetailDefinition::validate`]，错误载荷即 `vdfs/write`
//!   的字段级失败载荷（[`VdfsValidationError`]）。
//!
//! 能力判定不在本模块：可写性来自 **VDFS 访问位**，可新建 / 可导入来自 provider 的
//! `root_access` / `root_new_types`，可测试与否来自 [`DetailDefinition::actions`]
//! 里声明的动作。

use crate::symbio_core::vdfs_provider::{VdfsFieldError, VdfsValidationError};
use serde::{Deserialize, Serialize};

// ==================== 详情页定义（definition-driven detail） ====================
//
// 交互不复杂的详情页由后端下发**定义**、前端通用渲染器（DetailForm）动态生成，
// 前端零页面开发。定义能力：预设联动填充 / 动态候选（datalist/select）/
// 密码显隐 / 数字范围 / 折叠分区 / 条件徽标与动作 / id·name 派生回落链。
// 只读概览型详情（如 agent 概览）由 `info` 绑定表达；
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
    /// 候选项说明（次要说明文字，如「中风险及以下自动执行；高风险需审批」）。
    ///
    /// 纵向表单渲染器可忽略；紧凑渲染形态（如会话选项栏的候选菜单）据此给出
    /// 每个候选项的一行解释——与 [`DetailField::description`] 同一分工，
    /// 只是作用在候选项而非字段上。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 机制原生取值原语（**闭集**）——前端实现的通用取值能力，不含任何业务语义。
///
/// 后端无法唤起原生对话框，故字段可声明一个原语：前端先取值、写入本字段，再提交。
/// 闭集的**唯一定义处**是前端 `schemas/vdfs-form.ts::DETAIL_PICKS`，由
/// `protocol-mirror-audit` 的 C 组按 `DETAIL_PICK_` 前缀提取本组取值逐词比对。
pub const DETAIL_PICK_DIRECTORY: &str = "directory";
pub const DETAIL_PICK_FILE: &str = "file";

/// 表单字段定义。`widget` ∈ text | password | number | select | textarea |
/// toggle | datalist | list | map | static | path | form；`options`/`suggestions`
/// 为静态候选，`*_from_preset` 为真时候选来自当前预设的 `options[key]`
/// （如 provider 预设注入模型列表）。
///
/// 结构化 widget 的表单模型约定（渲染器与 `validate_manifest` 两侧一致）：
/// - `list`：字符串数组，编辑态每行一项；
/// - `map`：字符串键值对，编辑态每行 `KEY=VALUE`；
/// - `static`：只读展示（info 绑定），值来自 `item.config`/`extra`，
///   `options` 可作值→标签映射；
/// - `path`：单行文本 + 原生选择入口（配 [`DetailField::pick`]）；
/// - `form`：**结构化子对象**，形状由 [`DetailField::form`] 的子定义描述。
///
/// ## 字段级与动作级的能力对齐
///
/// `disabled_when` / `icon` 曾只在 [`DetailAction`] 上有，字段只能表达「显 / 隐」。
/// 但「可见但不可改」（锁定字段、只读派生字段）是普遍需求，把它表达成「隐藏」
/// 是错的——用户会以为这一项不存在。故与动作对齐补上。
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
    /// 图标名（纯 UI 映射；缺省不显示图标）。
    ///
    /// 纵向表单渲染器可忽略它；紧凑渲染形态（如会话选项栏）据此给每个字段一个标识。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 条件显隐（不满足时整行不渲染；求值同徽标/动作条件）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<DetailCondition>,
    /// 禁用条件：**成立才禁用**（缺省 = 不禁用）。与 [`DetailAction::disabled_when`] 同义。
    ///
    /// 与 `visible_when` 的分工：隐藏 = 「这一项与当前场景无关」；
    /// 禁用 = 「这一项存在、但此刻不能改」（并在 `description` 说明原因）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_when: Option<DetailCondition>,
    /// 机制原生取值原语（闭集，见 [`DETAIL_PICK_DIRECTORY`] / [`DETAIL_PICK_FILE`]）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
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
    /// 候选项（`widget = "select"` 时渲染为候选菜单）。
    ///
    /// 对**没有候选菜单**的 widget（`path` / `form`），它退化为一张**值→标签表**：
    /// 紧凑渲染形态（会话选项栏）按它把当前值压成一句话（前端
    /// `schemas/vdfs-form.compactFieldText`）。例：`path` 字段给
    /// `{value: "", label: "未选择目录"}` 以表达「未设置」，`form` 字段给
    /// `{value: "true", label: "已开启"}` 以表达子对象的开关态。
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
    /// `widget = "form"`：本字段值是**结构化子对象**，由这份子定义描述其字段。
    ///
    /// 与「另开一个 `form` 节点」的差别：子对象仍是**本表单模型的一个键**，
    /// 保存时随外层一次提交，不会产生第二个写入入口。
    ///
    /// 紧凑渲染形态（会话选项栏）要在按钮上显示这个子对象的一句话摘要，取值规则：
    /// 按子定义的 `title_from` 链从子对象取一个**代表值** ⇒ 本字段 `options`
    /// 非空时按「值→标签」查表（`String(代表值)` 匹配，见 [`DetailOption`]）
    /// ⇒ 仍无则回落 [`DetailField::label`]。因此子定义应声明 `title_from`
    /// 指向那个「一眼能看出状态」的子字段（如心跳的 `enabled`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Box<DetailDefinition>>,
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
/// （机制语义动作，前端接统一通道）或自定义（预留）；`payload` 合并进 save 负载
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
    /// 空格分隔的 class 修饰词：`primary` / `secondary`（默认观感，无专属规则）/ `danger` /
    /// `icon`（图标按钮，可叠加 → `icon danger`）；单值 `divider` 渲染为分隔线而非按钮
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

/// 详情页定义。`binding` ∈ upload（资源：预填节点内容，保存走
/// manifest 写入）| option（可选项：字段直接落在表单模型上，保存走节点写入）
/// | info（只读概览：无保存，字段取值来自节点 attributes，配 `static` widget 展示）。
/// 派生链均为「首个非空」：
/// `title_from` 生成标题，`name_from` 保存时补名称，`id_from` 新建时
/// 派生 slug id（前端去重 `-2` 递增，后端 `validate_manifest` 兜底）。
///
/// 校验同源：提交值的合法性由定义自己判定（[`DetailDefinition::validate`]），
/// 使用方不再各写一套字段规则。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DetailDefinition {
    pub binding: String,
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

// ==================== 定义构造器 ====================
//
// 字段与表单骨架的**最小构造**。定义由**资源的拥有者**产出（如每个插件产出
// 自己的配置定义），因此这些形状要在多处复用——写在 schema 侧一次，
// 好过在每个使用方各抄一遍（抄出来的副本必然随字段增删而漂移）。

impl DetailField {
    /// 公共骨架：`description` 为空串即「无说明」（避免下发 `Some("")` 让前端渲染空行）
    fn base(key: &str, label: &str, widget: &str, description: &str) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            widget: widget.into(),
            description: (!description.is_empty()).then(|| description.to_string()),
            ..Default::default()
        }
    }

    /// 补说明（构造器缺省无说明时用；空串视为无说明）
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        let d = description.into();
        self.description = (!d.is_empty()).then_some(d);
        self
    }

    /// 数字字段（`min` / `max` 与新建态缺省值一并声明）
    pub fn number(
        key: &str,
        label: &str,
        description: &str,
        min: f64,
        max: f64,
        default: serde_json::Value,
    ) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
            step: Some(1.0),
            default: Some(default),
            ..Self::base(key, label, "number", description)
        }
    }

    /// 开关字段
    pub fn toggle(key: &str, label: &str, description: &str, default: bool) -> Self {
        Self {
            default: Some(serde_json::Value::Bool(default)),
            ..Self::base(key, label, "toggle", description)
        }
    }

    /// 密码字段（前端显隐切换）
    pub fn password(key: &str, label: &str, description: &str, placeholder: &str) -> Self {
        Self {
            placeholder: Some(placeholder.to_string()),
            ..Self::base(key, label, "password", description)
        }
    }

    /// 单行文本字段
    pub fn text(key: &str, label: &str, description: &str) -> Self {
        Self::base(key, label, "text", description)
    }

    /// 下拉字段（静态候选）
    pub fn select(key: &str, label: &str, options: Vec<DetailOption>, default: &str) -> Self {
        Self {
            options,
            default: Some(serde_json::Value::String(default.to_string())),
            ..Self::base(key, label, "select", "")
        }
    }
}

impl DetailDefinition {
    /// 单分区表单骨架：`binding = option` + 一个「保存配置」动作。
    ///
    /// `binding = option` 是 **VDFS 宿主表单**的通用语义——「取值来自外部
    /// （`vdfs/read`）、保存只交回纯字段值（`vdfs/write`）」；`upload` / `info`
    /// 是另两种宿主形态，不适用本骨架。
    pub fn form(title: impl Into<String>, fields: Vec<DetailField>) -> Self {
        Self {
            binding: "option".into(),
            title_fallback: Some(title.into()),
            sections: vec![DetailSection {
                title: None,
                collapsed: false,
                fields,
            }],
            actions: vec![DetailAction {
                id: "save".into(),
                label: "保存配置".into(),
                style: "primary".into(),
                busy_label: Some("保存中…".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
}

// ==================== 定义自带校验 ====================
// **定义与校验同源**：字段声明在哪里，字段的合法性判定就在哪里。
// 使用方（配置 provider / 上传 provider）把提交值交给定义即可，不必各自
// 复写字段规则——这是「定义驱动表单」成立的前提。
//
// 结果复用 VDFS 的字段级错误载荷 [`VdfsValidationError`]：定义本就是 VDFS 上
// 的一种宿主方言（`ext = form`），而 `vdfs/write` 的失败载荷正是它的输出格式。

/// 空值判定：`null` 与「全空白字符串」视为空；数字 / 布尔 / 容器不算空
/// （`false` / `0` 是**有效值**，不能当成「没填」）。
fn is_blank(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

impl DetailCondition {
    /// 以 `value` 为表单模型求值，判断本条件是否成立。
    ///
    /// **空条件恒成立**——这是条件字段的中性语义（见本类型文档），
    /// 也是前端渲染器与后端校验保持一致的关键：`when` / `visible_when`
    /// 缺省即显示，因此后端不能把「无条件」当成「不成立」。
    pub fn holds(&self, value: &serde_json::Value) -> bool {
        if !self.all.iter().all(|c| c.holds(value)) {
            return false;
        }
        let cur = value.get(&self.key);
        if let Some(eq) = &self.equals {
            if cur != Some(eq) {
                return false;
            }
        }
        if let Some(ne) = &self.not_equals {
            if cur == Some(ne) {
                return false;
            }
        }
        if let Some(t) = self.truthy {
            if cur.and_then(serde_json::Value::as_bool).unwrap_or(false) != t {
                return false;
            }
        }
        true
    }
}

impl DetailField {
    /// 单字段类型 / 范围校验（`None` = 通过）。
    ///
    /// 覆盖 `widget` 声明的形状约束与 `min` / `max` / `options` 声明的取值约束；
    /// `required` 与条件显隐由 [`DetailDefinition::validate`] 统一处理（它们
    /// 取决于「有没有提交」而非「值本身合不合法」）。
    pub fn check(&self, v: &serde_json::Value) -> Option<String> {
        match self.widget.as_str() {
            "number" => {
                let Some(n) = v.as_f64() else {
                    return Some("必须是数字".to_string());
                };
                if let Some(min) = self.min {
                    if n < min {
                        return Some(format!("不能小于 {min}"));
                    }
                }
                if let Some(max) = self.max {
                    if n > max {
                        return Some(format!("不能大于 {max}"));
                    }
                }
                None
            }
            "toggle" => (!v.is_boolean()).then(|| "必须是布尔值".to_string()),
            "select" => {
                let Some(s) = v.as_str() else {
                    return Some("必须是字符串".to_string());
                };
                if !self.options.is_empty() && !self.options.iter().any(|o| o.value == s) {
                    return Some(format!(
                        "必须是以下之一：{}",
                        self.options
                            .iter()
                            .map(|o| o.value.as_str())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    ));
                }
                None
            }
            "list" => (!v.is_array()).then(|| "必须是字符串数组".to_string()),
            "map" => (!v.is_object()).then(|| "必须是键值对象".to_string()),
            // 结构化子对象：形状由子定义逐字段校验（见 [`DetailDefinition::validate`]）
            "form" => (!v.is_object()).then(|| "必须是键值对象".to_string()),
            "text" | "password" | "textarea" | "datalist" | "path" => {
                (!v.is_string()).then(|| "必须是字符串".to_string())
            }
            _ => None,
        }
    }
}

impl DetailDefinition {
    /// 按定义逐字段校验提交值；有错则返回**字段级**错误（供前端逐字段高亮）。
    ///
    /// 与前端渲染保持一致的两条豁免：`static` 只读字段不参与校验；
    /// `visible_when` 不成立的隐藏字段不参与校验（没显示的字段不该拦提交）。
    pub fn validate(&self, value: &serde_json::Value) -> Result<(), VdfsValidationError> {
        let Some(obj) = value.as_object() else {
            return Err(VdfsValidationError::new("提交内容必须是 JSON 对象"));
        };
        let mut err = VdfsValidationError::new("校验未通过");

        for section in &self.sections {
            for f in &section.fields {
                // 只读展示字段不参与校验
                if f.widget == "static" {
                    continue;
                }
                // 条件隐藏字段不参与校验（与前端渲染保持一致）
                if let Some(cond) = &f.visible_when {
                    if !cond.holds(value) {
                        continue;
                    }
                }
                let Some(current) = obj.get(&f.key) else {
                    if f.required {
                        err.fields.push(VdfsFieldError {
                            field: f.key.clone(),
                            message: "必填项缺失".to_string(),
                        });
                    }
                    continue;
                };
                if f.required && is_blank(current) {
                    err.fields.push(VdfsFieldError {
                        field: f.key.clone(),
                        message: "必填项不能为空".to_string(),
                    });
                    continue;
                }
                if let Some(message) = f.check(current) {
                    err.fields.push(VdfsFieldError {
                        field: f.key.clone(),
                        message,
                    });
                    continue;
                }
                // 结构化子对象：**递归**用子定义校验，错误字段带父前缀
                // （`heartbeat.interval_seconds`），前端因此仍能逐字段高亮。
                // 校验同源：子定义的规则不在使用方复写一遍。
                if let Some(sub) = &f.form {
                    if current.is_object() {
                        if let Err(sub_err) = sub.validate(current) {
                            for fe in sub_err.fields {
                                err.fields.push(VdfsFieldError {
                                    field: format!("{}.{}", f.key, fe.field),
                                    message: fe.message,
                                });
                            }
                        }
                    }
                }
            }
        }

        if err.has_fields() {
            Err(err)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field(key: &str, widget: &str) -> DetailField {
        DetailField {
            key: key.into(),
            label: key.into(),
            widget: widget.into(),
            ..Default::default()
        }
    }

    fn def_of(fields: Vec<DetailField>) -> DetailDefinition {
        DetailDefinition {
            sections: vec![DetailSection {
                title: None,
                collapsed: false,
                fields,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn required_missing_and_blank_are_reported_per_field() {
        let mut f = field("host", "text");
        f.required = true;
        let def = def_of(vec![f]);

        assert_eq!(
            def.validate(&json!({})).unwrap_err().fields[0].field,
            "host"
        );
        assert_eq!(
            def.validate(&json!({ "host": "   " })).unwrap_err().fields[0].field,
            "host"
        );
        // `false` / `0` 是有效值，不是「没填」
        assert!(def.validate(&json!({ "host": "127.0.0.1" })).is_ok());
    }

    #[test]
    fn non_object_payload_is_rejected() {
        assert!(def_of(vec![]).validate(&json!("nope")).is_err());
    }

    /// 隐藏字段（`visible_when` 不成立）与只读展示字段都不参与校验：
    /// 没显示的字段不该拦住提交。
    #[test]
    fn hidden_and_static_fields_are_exempt() {
        let mut hidden = field("port", "number");
        hidden.required = true;
        hidden.visible_when = Some(DetailCondition {
            key: "enabled".into(),
            equals: Some(json!(true)),
            ..Default::default()
        });
        let mut ro = field("status", "static");
        ro.required = true;

        let def = def_of(vec![hidden, ro]);

        // 未启用 → 端口整行不渲染 → 缺失也不算错
        assert!(def.validate(&json!({ "enabled": false })).is_ok());
        // 启用 → 端口回归校验
        assert_eq!(
            def.validate(&json!({ "enabled": true }))
                .unwrap_err()
                .fields[0]
                .field,
            "port"
        );
    }

    /// 空条件恒成立（`when` / `visible_when` 缺省即显示）。
    #[test]
    fn empty_condition_always_holds() {
        assert!(DetailCondition::default().holds(&json!({})));
    }

    #[test]
    fn field_check_covers_shape_and_range() {
        let mut n = field("port", "number");
        n.min = Some(1.0);
        n.max = Some(65535.0);
        assert!(n.check(&json!(80)).is_none());
        assert!(n.check(&json!(0)).is_some());
        assert!(n.check(&json!(70000)).is_some());
        assert!(n.check(&json!("abc")).is_some());

        let mut s = field("proto", "select");
        s.options = vec![
            DetailOption {
                value: "http".into(),
                label: "HTTP".into(),
                description: None,
            },
            DetailOption {
                value: "https".into(),
                label: "HTTPS".into(),
                description: None,
            },
        ];
        assert!(s.check(&json!("http")).is_none());
        assert!(s.check(&json!("carrier-pigeon")).is_some());

        assert!(field("flag", "toggle").check(&json!(true)).is_none());
        assert!(field("flag", "toggle").check(&json!("yes")).is_some());
        assert!(field("hosts", "list").check(&json!(["a"])).is_none());
        assert!(field("hosts", "list").check(&json!("a")).is_some());
        assert!(field("env", "map").check(&json!({ "K": "V" })).is_none());
        assert!(field("env", "map").check(&json!([])).is_some());
        assert!(field("dir", "path").check(&json!("/tmp")).is_none());
        assert!(field("dir", "path").check(&json!(1)).is_some());
        assert!(field("hb", "form").check(&json!({})).is_none());
        assert!(field("hb", "form").check(&json!("nope")).is_some());
    }

    /// 结构化子对象（`widget = "form"`）用子定义**递归**校验，错误字段带父前缀。
    #[test]
    fn nested_form_field_validates_against_sub_definition() {
        let mut interval = field("interval_seconds", "number");
        interval.min = Some(10.0);
        let mut hb = field("heartbeat", "form");
        hb.form = Some(Box::new(def_of(vec![interval])));
        let def = def_of(vec![hb]);

        assert!(def
            .validate(&json!({ "heartbeat": { "interval_seconds": 30 } }))
            .is_ok());
        // 错误字段名带父前缀，前端仍能逐字段高亮
        assert_eq!(
            def.validate(&json!({ "heartbeat": { "interval_seconds": 1 } }))
                .unwrap_err()
                .fields[0]
                .field,
            "heartbeat.interval_seconds"
        );
    }

    /// `disabled_when` 成立**不**豁免校验：字段仍然在提交值里（只是用户改不动），
    /// 与 `visible_when` 不成立的「没显示就不该拦提交」是两回事。
    #[test]
    fn disabled_field_is_still_validated() {
        let mut f = field("port", "number");
        f.required = true;
        f.disabled_when = Some(DetailCondition {
            key: "locked".into(),
            equals: Some(json!(true)),
            ..Default::default()
        });
        let def = def_of(vec![f]);
        assert_eq!(
            def.validate(&json!({ "locked": true })).unwrap_err().fields[0].field,
            "port"
        );
    }
}
