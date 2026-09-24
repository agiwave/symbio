//! Skill 详情页定义（definition-driven detail）
//!
//! Skill 条目的持久化形态是 `<目录名>/SKILL.md`（YAML frontmatter + Markdown
//! body，解析权威在 `super::super::loader::parse_skill_file`）。本模块提供：
//! - 表单定义：frontmatter 字段 + body 的完整表单（upload 绑定）；
//! - 双向映射：表单 manifest → SKILL.md（`validate_manifest`，plugin.rs），
//!   SKILL.md → 表单预填（本模块的 [`skill_md_to_config`]，经 `vdfs/read` 下发）。
//!
//! 表单字段键与 frontmatter 键一致（`allowed_tools`/`disable_model_invocation`
//! 保存时映射为行业键 `allowedTools`/`disable-model-invocation`）。
//! BUG-SR6 硬约束（目录名 == frontmatter name）由 `manifest_to_skill_md` 强制。

use crate::symbio_core::schemas::detail::{
    DetailAction, DetailCondition, DetailDefinition, DetailField, DetailSection,
};

fn field(key: &str, label: &str, desc: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: widget.into(),
        ..Default::default()
    }
}

/// Skill 详情页定义（纯静态结构，开销可忽略）
pub fn skill_detail_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "upload".into(),
        title_from: vec!["name".into()],
        title_fallback: Some("新建 Skill".into()),
        subtitle_from: vec!["description".into()],
        name_from: vec!["name".into()],
        id_from: vec!["name".into()],
        sections: vec![
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![
                    DetailField {
                        required: true,
                        placeholder: Some("例如：pdf-extract".into()),
                        ..field(
                            "name",
                            "名称",
                            "即条目目录名（ID），保存时必须与目录名一致",
                            "text",
                        )
                    },
                    DetailField {
                        required: true,
                        rows: Some(3),
                        full_width: true,
                        placeholder: Some("这个 Skill 做什么、何时使用（至少 10 字符）".into()),
                        ..field(
                            "description",
                            "描述",
                            "展示给用户与 LLM 的用途说明",
                            "textarea",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("触发与调用".into()),
                collapsed: true,
                fields: vec![
                    DetailField {
                        rows: Some(3),
                        full_width: true,
                        ..field(
                            "when_to_use",
                            "何时使用",
                            "面向 LLM 的使用时机补充说明",
                            "textarea",
                        )
                    },
                    DetailField {
                        placeholder: Some("例如：<input_file>".into()),
                        ..field(
                            "argument_hint",
                            "参数提示",
                            "Skill 需要参数时的占位提示；留空 = 无参调用",
                            "text",
                        )
                    },
                    DetailField {
                        rows: Some(2),
                        ..field(
                            "allowed_tools",
                            "允许工具",
                            "每行一个工具名；留空 = 不限制",
                            "list",
                        )
                    },
                    DetailField {
                        placeholder: Some("例如：claude-sonnet-4-5".into()),
                        ..field(
                            "model",
                            "指定模型",
                            "执行此 Skill 时使用的模型；留空 = 跟随会话",
                            "text",
                        )
                    },
                    DetailField {
                        default: Some(serde_json::json!(false)),
                        ..field(
                            "disable_model_invocation",
                            "禁止模型自主调用",
                            "开启后仅能由用户显式触发",
                            "toggle",
                        )
                    },
                ],
            },
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![DetailField {
                    required: true,
                    rows: Some(14),
                    full_width: true,
                    placeholder: Some("Skill 指令正文（Markdown），${var} 会被调用参数替换".into()),
                    ..field(
                        "body",
                        "内容（Markdown）",
                        "SKILL.md frontmatter 之后的正文",
                        "textarea",
                    )
                }],
            },
        ],
        presets: None,
        badges: vec![],
        actions: vec![
            DetailAction {
                id: "save".into(),
                label: "保存".into(),
                style: "primary".into(),
                busy_label: Some("保存中…".into()),
                ..Default::default()
            },
            // 「导出」：VDFS 节点动作 `export`（vdfs/action）→ provider 的
            // `export_zip`；与「新建类型 zip」的导入互为逆向
            DetailAction {
                id: crate::symbio_core::vdfs_provider::VDFS_ACTION_EXPORT.into(),
                label: "导出整包".into(),
                style: "secondary".into(),
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("打包中…".into()),
                ..Default::default()
            },
            DetailAction {
                id: "divider".into(),
                label: String::new(),
                style: "divider".into(),
                ..Default::default()
            },
            DetailAction {
                id: "delete".into(),
                label: "删除 Skill".into(),
                style: "icon danger".into(),
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("删除中…".into()),
                ..Default::default()
            },
        ],
    }
}

/// 解析 SKILL.md →（frontmatter YAML、body）；无 frontmatter 时返回 None。
/// 分隔规则与 loader 的 `parse_skill_file` 保持一致。
pub fn parse_skill_md(content: &str) -> Option<(serde_yaml_ng::Value, String)> {
    let content = content.replace("\r\n", "\n");
    let rest = content.strip_prefix("---\n")?;
    let (yaml_str, body) = rest.split_once("\n---")?;
    let yaml = serde_yaml_ng::from_str(yaml_str).ok()?;
    let body = body.strip_prefix('\n').unwrap_or(body);
    Some((yaml, body.trim().to_string()))
}

/// 表单 manifest → SKILL.md 全文（`validate_manifest` 用）。
///
/// 约束（与 loader 一致，保存时即给出明确错误）：
/// - BUG-SR6：目录名必须 == frontmatter `name`（`id` 为权威）；
/// - BUG-SR7：description 至少 10 字符；
/// - name / description 必填。
pub fn manifest_to_skill_md(
    id: &str,
    manifest: &serde_json::Value,
) -> Result<String, crate::symbio_core::PluginError> {
    use crate::symbio_core::PluginError;

    let get_str = |key: &str| -> Option<String> {
        manifest
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
    };

    let name = get_str("name").unwrap_or_default();
    if name.is_empty() {
        return Err(PluginError::ValidationError(
            "Skill 的 name 不能为空".to_string(),
        ));
    }
    if name != id {
        return Err(PluginError::ValidationError(format!(
            "BUG-SR6: Skill 名称 '{name}' 必须与目录名（ID）'{id}' 一致"
        )));
    }
    let description = get_str("description").unwrap_or_default();
    if description.chars().count() < 10 {
        return Err(PluginError::ValidationError(
            "BUG-SR7: description 至少需要 10 字符（说明 Skill 的用途与使用时机）".to_string(),
        ));
    }

    let mut fm = serde_yaml_ng::Mapping::new();
    let mut insert = |k: &str, v: serde_yaml_ng::Value| {
        fm.insert(serde_yaml_ng::Value::String(k.into()), v);
    };
    insert("name", serde_yaml_ng::Value::String(name));
    insert("description", serde_yaml_ng::Value::String(description));
    if let Some(v) = get_str("when_to_use").filter(|s| !s.is_empty()) {
        insert("when_to_use", serde_yaml_ng::Value::String(v));
    }
    if let Some(v) = get_str("argument_hint").filter(|s| !s.is_empty()) {
        insert("argument_hint", serde_yaml_ng::Value::String(v));
    }
    // 表单 list 控件 → 行业键 allowedTools
    let tools: Vec<String> = manifest
        .get("allowed_tools")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if !tools.is_empty() {
        insert(
            "allowedTools",
            serde_yaml_ng::Value::Sequence(
                tools
                    .into_iter()
                    .map(serde_yaml_ng::Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(v) = get_str("model").filter(|s| !s.is_empty()) {
        insert("model", serde_yaml_ng::Value::String(v));
    }
    if manifest
        .get("disable_model_invocation")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        insert("disable-model-invocation", serde_yaml_ng::Value::Bool(true));
    }

    let yaml = serde_yaml_ng::to_string(&serde_yaml_ng::Value::Mapping(fm))
        .map_err(|e| PluginError::InternalError(format!("序列化 frontmatter 失败: {e}")))?;
    let body = get_str("body").unwrap_or_default();

    Ok(format!("---\n{yaml}---\n{body}\n"))
}

/// SKILL.md → 预填 config（表单字段键；`vdfs/read` 把它作为 JSON 正文下发）
pub fn skill_md_to_config(content: &str) -> Option<serde_json::Value> {
    let (yaml, body) = parse_skill_md(content)?;
    let get_str = |key: &str| -> Option<String> {
        yaml.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let mut cfg = serde_json::Map::new();
    if let Some(v) = get_str("name") {
        cfg.insert("name".into(), serde_json::json!(v));
    }
    if let Some(v) = get_str("description") {
        cfg.insert("description".into(), serde_json::json!(v));
    }
    if let Some(v) = get_str("when_to_use") {
        cfg.insert("when_to_use".into(), serde_json::json!(v));
    }
    if let Some(v) = get_str("argument_hint") {
        cfg.insert("argument_hint".into(), serde_json::json!(v));
    }
    if let Some(v) = yaml.get("allowedTools").and_then(|v| v.as_sequence()) {
        let tools: Vec<String> = v
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        cfg.insert("allowed_tools".into(), serde_json::json!(tools));
    }
    if let Some(v) = get_str("model") {
        cfg.insert("model".into(), serde_json::json!(v));
    }
    if let Some(v) = yaml
        .get("disable-model-invocation")
        .and_then(|v| v.as_bool())
    {
        cfg.insert("disable_model_invocation".into(), serde_json::json!(v));
    }
    cfg.insert("body".into(), serde_json::json!(body));
    Some(serde_json::Value::Object(cfg))
}

#[cfg(test)]
#[path = "detail.test.rs"]
mod tests;
