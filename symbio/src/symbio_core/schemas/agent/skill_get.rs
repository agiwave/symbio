//! `skill/get` 路由的请求/响应结构
//!
//! BUG-FR9：在 SkillView 详情面板展示 body 预览，避免用户必须打开文件才能看到内容。
//! body 已经在 list_skills 时加载并存储，get 直接复用即可（O(1) 查找）。

use serde::{Deserialize, Serialize};

/// 获取指定 skill 的详情请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// skill 名称
    pub name: String,
    /// 可选：覆盖 workdir
    #[serde(default)]
    pub workdir: Option<String>,
}

/// skill 详情响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// skill 名称
    pub name: String,
    /// skill 描述
    pub description: String,
    /// SKILL.md 绝对路径
    pub file_path: String,
    /// 来源分类（workspace / system / external / unknown）
    pub source: String,
    /// 是否需要参数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// 使用场景
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// BUG-FR9：SKILL.md 的 body 内容（用于详情面板预览）
    pub body: String,
    /// BUG-FR9：body 长度（字符数），便于前端做"展开/收起"判断
    pub body_chars: usize,
    /// BUG-FR9：是否被 max_body_chars 截断
    pub body_truncated: bool,
}
