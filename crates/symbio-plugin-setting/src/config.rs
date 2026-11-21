//! setting 插件的配置与表单定义（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 三组字段，一个判据：**有没有消费者**
//!
//! | 组 | 字段 | 消费者 |
//! |---|---|---|
//! | 档案 | `display_name` / `description` | 系统提示词（[`super::segment`]）+ 界面自述 |
//! | 偏好 | `reply_language` / `verbosity` | 系统提示词（[`super::segment`]） |
//! | 指令 | `agents_file_name` | 两级 AGENTS.md 的读取（[`super::plugin`] 经作用域 provider） |
//!
//! 只收**有消费者**的字段：图标、默认模型、默认工作区这类东西看起来也属于「智能体
//! 的设置」，但它们的消费者在别的插件（前端图标表 / `model` / `work`）里，本插件
//! 交出去也没人读——那就是死配置（用户填了、什么都没发生）。等消费方接好了再加，
//! 不预先发明字段。
//!
//! ## 指令文件名可由用户配置，但内容不由本插件设置
//!
//! AGENTS.md 是**稳定指令 / 人格**（`{workdir}` 与 `{agentdir}` 各一份，工作区优先
//! 覆盖智能体级），与 `memory` 的 `MEMORY.md`（动态记忆）语义隔离。本插件只配
//! **文件名**（默认 `AGENTS.md`）——正文通过作用域 VDFS provider 现读，不在配置里
//! 存一份（存了就等于「设置记忆内容」，正是要拆开的职责）。
//!
//! ## 缺省全空 = 不改变任何一轮的提示词
//!
//! 档案 / 偏好四个字段的缺省值都是「未设置」（空串），因此**刚装配完不会注入档案
//! 段**。这不是偷懒：本插件描述的是「这个智能体是谁」，宿主不该替用户编一个名字。
//! 指令段则由**文件是否存在**决定，与这四个字段无关。

use symbio_plugin_api::{DetailDefinition, DetailField, DetailOption, DetailSection};

/// setting 插件配置 —— 本智能体自身的**档案**与**回答偏好**
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SettingConfig {
    /// 显示名（空 = 未设置，界面回落目录名、提示词不提）
    #[serde(default)]
    pub display_name: String,

    /// 一句话简介（空 = 未设置）
    #[serde(default)]
    pub description: String,

    /// 回复语言（空 = 跟随用户，不约束）
    ///
    /// 自由文本而非枚举：语言名是开放集合，写死一张表只会让「用户想用的那种」不在
    /// 表里。它直接进提示词，模型看得懂任何写法。
    #[serde(default)]
    pub reply_language: String,

    /// 回答详略（`""` / `concise` / `standard` / `detailed`；空 = 不约束）
    #[serde(default)]
    pub verbosity: String,

    /// 两级指令文件的名字（工作区级 / 智能体级同名；默认 `AGENTS.md`）。
    ///
    /// 只配**文件名**，不配内容：正文经作用域 VDFS provider 现读，稳定指令与动态
    /// 记忆（`memory` 的 `MEMORY.md`）因此物理隔离。留空 / 非法文件名回落到默认值。
    #[serde(default = "default_agents_file_name")]
    pub agents_file_name: String,
}

/// 指令文件名缺省值（`AGENTS.md` —— 行业通行约定）。
fn default_agents_file_name() -> String {
    "AGENTS.md".to_string()
}

impl SettingConfig {
    /// 显示名（**去空白**；空白视为未设置）
    pub fn display_name(&self) -> &str {
        self.display_name.trim()
    }

    /// 简介（去空白）
    pub fn description(&self) -> &str {
        self.description.trim()
    }

    /// 回复语言（去空白）
    pub fn reply_language(&self) -> &str {
        self.reply_language.trim()
    }

    /// 回答详略（去空白；不在取值表里的写法按「不约束」处理）
    ///
    /// 手写的 `PLUGIN.yml` 可能写错，而「写错」不该变成一句模型看不懂的指令——
    /// 认不出的值按未设置处理，用户改一次就好。
    pub fn verbosity(&self) -> Option<&'static str> {
        match self.verbosity.trim() {
            "concise" => Some("简洁"),
            "standard" => Some("标准"),
            "detailed" => Some("详细"),
            _ => None,
        }
    }

    /// 指令文件名（去空白；空白 / 含路径分隔符 → 回落到 `AGENTS.md`）。
    ///
    /// 只接受**纯文件名**：作用域 provider 的根已是工作区 / 智能体目录，带 `/` 或
    /// `..` 的名字是想跳出作用域——拒绝并回落默认值，不让它成为逃逸向量。
    pub fn agents_file_name(&self) -> &str {
        let name = self.agents_file_name.trim();
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name == "."
        {
            "AGENTS.md"
        } else {
            name
        }
    }

    /// 有没有任何一项**档案 / 偏好**被设置过（全空 → 不注入档案段）。
    ///
    /// 指令文件名不在此列：它有默认值，且指令段的注入由文件是否存在决定。
    pub fn is_empty(&self) -> bool {
        self.display_name().is_empty()
            && self.description().is_empty()
            && self.reply_language().is_empty()
            && self.verbosity().is_none()
    }
}

/// 详略的候选（表单与说明同源，不写第二份字面量）
const VERBOSITY_OPTIONS: [(&str, &str); 4] = [
    ("", "不约束"),
    ("concise", "简洁"),
    ("standard", "标准"),
    ("detailed", "详细"),
];

/// 配置表单定义 —— **定义由配置的拥有者产出**
///
/// 两组字段分区呈现（档案 / 偏好）：它们回答的是两个不同的问题，
/// 挤在一个无标题分区里会让用户看不出「下面这两个是偏好」。
pub fn config_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "option".to_string(),
        title_fallback: Some("智能体".to_string()),
        sections: vec![
            DetailSection {
                title: Some("指令文件".to_string()),
                collapsed: false,
                fields: vec![DetailField::text(
                    "agents_file_name",
                    "指令文件名",
                    "两级指令文件的名字（默认 AGENTS.md）；工作区级优先覆盖智能体级，正文由该文件提供",
                )],
            },
            DetailSection {
                title: Some("档案".to_string()),
                collapsed: false,
                fields: vec![
                    DetailField::text(
                        "display_name",
                        "显示名",
                        "本智能体在界面上的名字；留空则沿用目录名",
                    ),
                    DetailField::text(
                        "description",
                        "简介",
                        "一句话说清本智能体是做什么的；每轮注入系统提示词",
                    ),
                ],
            },
            DetailSection {
                title: Some("回答偏好".to_string()),
                collapsed: false,
                fields: vec![
                    DetailField::text(
                        "reply_language",
                        "回复语言",
                        "如「中文」「English」；留空则跟随用户使用的语言",
                    ),
                    DetailField::select(
                        "verbosity",
                        "回答详略",
                        VERBOSITY_OPTIONS
                            .iter()
                            .map(|(value, label)| DetailOption {
                                value: (*value).to_string(),
                                label: (*label).to_string(),
                                description: None,
                            })
                            .collect(),
                        "",
                    ),
                ],
            },
        ],
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
