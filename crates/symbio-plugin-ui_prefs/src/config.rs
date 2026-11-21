//! 界面偏好配置：外观与提示音的**后端权威值**。
//!
//! 这份配置只收**有前端消费者**的字段（theme / font_size / 提示音三个维度）。
//! 「关于」页没有用户数据——版本来自编译期、引导状态是纯 UI 流程——故不落在这里，
//! 它的节点仍归本插件名下（见 `plugin_manager` 的分区归属），但正文恒为空。
//!
//! ## 默认值与前端同源
//!
//! `theme = light` / `font_size = medium` / 提示音全开 / `volume = 0.6` 与前端
//! `stores/appearance.ts`、`stores/soundSettings.ts`、`services/completionChime.ts` 的
//! 缺省一致。前端仍保留一份 localStorage 快照作首屏同步与离线降级（见 D-521），
//! 这里的值是**对齐后的权威**：两边不一致时以本配置为准。

use symbio_plugin_api::{DetailDefinition, DetailField, DetailOption, DetailSection};

/// 主题缺省（`light` / `dark` / `auto`）
pub const DEFAULT_THEME: &str = "light";
/// 字号缺省（`small` / `medium` / `large`）
pub const DEFAULT_FONT_SIZE: &str = "medium";
/// 提示音缺省音量，与前端 `DEFAULT_CHIME_VOLUME` 同源
pub const DEFAULT_VOLUME: f64 = 0.6;

fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}
fn default_font_size() -> String {
    DEFAULT_FONT_SIZE.to_string()
}
fn default_true() -> bool {
    true
}
fn default_volume() -> f64 {
    DEFAULT_VOLUME
}

/// 界面偏好 —— 外观 + 提示音
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UiPrefsConfig {
    /// 主题：`light` / `dark` / `auto`
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 字号：`small` / `medium` / `large`
    #[serde(default = "default_font_size")]
    pub font_size: String,

    /// 会话结束提示音总开关
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// 正常结束提示音
    #[serde(default = "default_true")]
    pub sound_completed: bool,
    /// 中止提示音
    #[serde(default = "default_true")]
    pub sound_aborted: bool,
    /// 失败提示音
    #[serde(default = "default_true")]
    pub sound_failed: bool,
    /// 提示音音量 0..1
    #[serde(default = "default_volume")]
    pub volume: f64,
}

impl Default for UiPrefsConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            font_size: default_font_size(),
            sound_enabled: default_true(),
            sound_completed: default_true(),
            sound_aborted: default_true(),
            sound_failed: default_true(),
            volume: default_volume(),
        }
    }
}

/// 主题候选（表单与说明同源）
const THEME_OPTIONS: [(&str, &str); 3] =
    [("light", "浅色"), ("dark", "深色"), ("auto", "跟随系统")];

/// 字号候选
const FONT_SIZE_OPTIONS: [(&str, &str); 3] = [("small", "小"), ("medium", "中"), ("large", "大")];

fn select_field(key: &str, label: &str, options: &[(&str, &str)]) -> DetailField {
    DetailField {
        key: key.to_string(),
        label: label.to_string(),
        widget: "select".to_string(),
        options: options
            .iter()
            .map(|(value, label)| DetailOption {
                value: (*value).to_string(),
                label: (*label).to_string(),
                description: None,
            })
            .collect(),
        ..Default::default()
    }
}

fn toggle_field(key: &str, label: &str) -> DetailField {
    DetailField {
        key: key.to_string(),
        label: label.to_string(),
        widget: "toggle".to_string(),
        ..Default::default()
    }
}

/// 配置定义：呈现与校验同源，写入经 `PluginConfigFile::apply` 落盘。
pub fn config_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "option".to_string(),
        title_fallback: Some("界面偏好".to_string()),
        sections: vec![
            DetailSection {
                title: Some("外观".to_string()),
                collapsed: false,
                fields: vec![
                    select_field("theme", "主题", &THEME_OPTIONS),
                    select_field("font_size", "字体大小", &FONT_SIZE_OPTIONS),
                ],
            },
            DetailSection {
                title: Some("提示音".to_string()),
                collapsed: false,
                fields: vec![
                    toggle_field("sound_enabled", "会话结束提示音"),
                    toggle_field("sound_completed", "正常结束"),
                    toggle_field("sound_aborted", "中止"),
                    toggle_field("sound_failed", "失败"),
                    DetailField {
                        key: "volume".to_string(),
                        label: "音量".to_string(),
                        widget: "number".to_string(),
                        min: Some(0.0),
                        max: Some(1.0),
                        step: Some(0.05),
                        ..Default::default()
                    },
                ],
            },
        ],
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
