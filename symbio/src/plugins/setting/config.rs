//! setting 插件的配置与表单定义（`<本插件目录>/PLUGIN.yml`）。
//!
//! ## 两组字段，一个判据：**有没有消费者**
//!
//! | 组 | 字段 | 消费者 |
//! |---|---|---|
//! | 档案 | `display_name` / `description` | 系统提示词（[`super::segment`]）+ 界面自述 |
//! | 偏好 | `reply_language` / `verbosity` | 系统提示词（[`super::segment`]） |
//!
//! 只收**有消费者**的字段：图标、默认模型、默认工作区这类东西看起来也属于「智能体
//! 的设置」，但它们的消费者在别的插件（前端图标表 / `model` / `work`）里，本插件
//! 交出去也没人读——那就是死配置（用户填了、什么都没发生）。等消费方接好了再加，
//! 不预先发明字段。
//!
//! ## 缺省全空 = 不改变任何一轮的提示词
//!
//! 四个字段的缺省值都是「未设置」（空串），因此**刚装配完什么都不会注入**。
//! 这不是偷懒：本插件描述的是「这个智能体是谁」，宿主不该替用户编一个名字。

use crate::symbio_core::{DetailDefinition, DetailField, DetailOption, DetailSection};

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

    /// 有没有任何一项被设置过（全空 → 不注入片段）
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
        title_fallback: Some("智能体设置".to_string()),
        sections: vec![
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
