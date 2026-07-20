//! `skill/list` 路由的请求/响应结构

use serde::{Deserialize, Serialize};

/// Skill 列表请求
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Request {
    /// 可选：覆盖 workdir（默认使用 ctx.workdir）
    #[serde(default)]
    pub workdir: Option<String>,
}

/// 单个 Skill 概览（用于列表展示，不含 body 全文）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// Skill 描述文件绝对路径
    pub file_path: String,
    /// 来源分类
    /// - `workspace`: 工作区级（`{workdir}/.symbio/skills`）
    /// - `system`: 系统级（`~/.symbio/plugins/skills`）
    /// - `external`: 第三方（`.qwen/skills`、`.sixth/skills`、`.qoder/skills`）
    pub source: String,
    /// 是否需要参数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// 使用场景
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
}

/// Skill 列表响应
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub skills: Vec<SkillInfo>,
}
