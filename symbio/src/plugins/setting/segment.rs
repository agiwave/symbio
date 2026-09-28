//! 配置 → **系统提示词片段**（本插件唯一的消费者）。
//!
//! ## 为什么必须注入
//!
//! 「我叫什么」「我说什么语言」这类信息只有被模型读到才有意义。落盘不进提示词
//! 等于用户填了表单、什么都没发生——那是死配置。因此本模块是本插件**唯一**的
//! 出口，也是那四个字段存在的全部理由。
//!
//! ## 为什么是全空则不注入
//!
//! 缺省状态（刚装配完）不该改变任何一轮的提示词：宿主没有替用户编名字的立场，
//! 每轮多背一行「本智能体：未命名」也只是烧 token。故 [`segment`] 在
//! [`SettingConfig::is_empty`] 时返回 `None`——**不产出空标题**。
//!
//! ## 为什么这么短
//!
//! 这段文字**每一轮**都进上下文，因此只写模型真正要用的东西：名字与简介各一次、
//! 两条偏好各一次，不加任何解释（「下面是你的名字」这种话对用户没用、对模型是噪音）。

use super::config::SettingConfig;

/// 系统提示词条目在收集器里的注册名（同名覆盖的键）
///
/// 子树里的注册会经 `agent` 插件的 `SubAgentVisitor` 加上 `agent/<id>/` 前缀，
/// 因此与系统侧的**同名**条目不冲突（并集）。
pub const SEGMENT_NAME: &str = "setting";

/// 片段的标题（渲染为 `【本智能体】`）
pub const SEGMENT_TITLE: &str = "本智能体";

/// 配置 → 系统提示词片段；**全空 → `None`**（不注入，也不产出空标题）。
///
/// 只写已设置的项：留空的字段不出现，模型因此不会去「猜」一个没填过的值。
pub fn segment(config: &SettingConfig) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    let name = config.display_name();
    if !name.is_empty() {
        parts.push(format!("名称：{name}"));
    }
    let description = config.description();
    if !description.is_empty() {
        parts.push(format!("简介：{description}"));
    }
    let language = config.reply_language();
    if !language.is_empty() {
        parts.push(format!("回复语言：{language}"));
    }
    if let Some(v) = config.verbosity() {
        parts.push(format!("回答详略：{v}"));
    }

    Some(format!("【{SEGMENT_TITLE}】{}", parts.join("；")))
}

#[cfg(test)]
#[path = "segment.test.rs"]
mod tests;
