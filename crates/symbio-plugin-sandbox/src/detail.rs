//! 沙箱档案详情页定义 + 会话选项字段（definition-driven detail）。
//!
//! 表单与 `SandboxProfileConfig` 同构：name/description 是展示元数据，
//! approval_mode/risk_threshold 承接原会话级 mode/risk_level（裁决 B），
//! allowed_facts/blocked_tools 是沙箱内部开关。前端只按 widget 通用渲染，
//! **不理解任何沙箱概念**——与 mcp / model 的详情走同一条通道。

use symbio_plugin_api::{DetailDefinition, DetailField, DetailOption, DetailSection};

/// 会话选项字段：`sandbox_id`（单选）。候选是档案 id → name，前端只渲染 select。
///
/// 这是 mode / risk_level 消失后**唯一的会话安全选项**——用户选一个沙箱，
/// 审批模式与风险阈值由该沙箱档案内部承载（裁决 B）。
pub(crate) fn sandbox_option_field(
    archives: &[(String, String)],
    active: Option<&str>,
) -> DetailField {
    let options: Vec<DetailOption> = if archives.is_empty() {
        vec![DetailOption {
            value: String::new(),
            label: "未配置".to_string(),
            description: Some("暂无沙箱，请前往「安全运行环境」添加".to_string()),
        }]
    } else {
        archives
            .iter()
            .map(|(id, name)| DetailOption {
                value: id.clone(),
                label: name.clone(),
                description: None,
            })
            .collect()
    };
    DetailField {
        key: "sandbox_id".to_string(),
        label: "沙箱".to_string(),
        description: Some("选择本次会话的安全运行环境（含审批策略）".to_string()),
        widget: "select".to_string(),
        icon: Some("sandbox".to_string()),
        default: active.map(|id| serde_json::json!(id)),
        options,
        ..Default::default()
    }
}

fn opt(value: &str, label: &str) -> DetailOption {
    DetailOption {
        value: value.into(),
        label: label.into(),
        description: None,
    }
}

fn field(key: &str, label: &str, desc: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: widget.into(),
        ..Default::default()
    }
}

/// 沙箱档案详情定义。
///
/// `binding = "upload"` + `title_from = ["name"]`：选中已有沙箱时右栏显示其名，
/// 新建草稿才回退「新建沙箱」（这是历史「标题恒为新建沙箱」缺陷的正式修法）。
pub(crate) fn sandbox_detail_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "upload".into(),
        title_from: vec!["name".into()],
        title_fallback: Some("新建沙箱".into()),
        name_from: vec!["name".into()],
        id_from: vec!["name".into()],
        sections: vec![
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![
                    DetailField {
                        required: true,
                        placeholder: Some("例如：strict（即档案 ID）".into()),
                        ..field("name", "名称", "用于生成档案 ID；内置档不可改名", "text")
                    },
                    DetailField {
                        rows: Some(3),
                        ..field(
                            "description",
                            "描述",
                            "这个沙箱做什么、适合什么场景",
                            "textarea",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("安全策略".into()),
                collapsed: false,
                fields: vec![
                    DetailField {
                        required: true,
                        options: vec![
                            opt("interactive", "交互模式（需审批时弹出审批卡）"),
                            opt("auto", "自动模式（无人值守，需审批时拒绝并提示）"),
                        ],
                        default: Some(serde_json::json!("interactive")),
                        ..field(
                            "approval_mode",
                            "审批模式",
                            "承接原「运行模式」选项；由沙箱内部承载，会话不再单独选择",
                            "select",
                        )
                    },
                    DetailField {
                        required: true,
                        options: vec![
                            opt("low", "低（放行大多数操作）"),
                            opt("medium", "中（默认）"),
                            opt("high", "高（更多操作需审批）"),
                        ],
                        default: Some(serde_json::json!("medium")),
                        ..field(
                            "risk_threshold",
                            "风险阈值",
                            "承接原「风险等级」选项；由沙箱内部承载，会话不再单独选择",
                            "select",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("能力约束（可选）".into()),
                collapsed: true,
                fields: vec![
                    DetailField {
                        rows: Some(3),
                        ..field(
                            "allowed_facts_json",
                            "事实白名单（JSON）",
                            "形如 {\"shell_command\": [\"git status\"]}；留空 = 不按事实限制",
                            "textarea",
                        )
                    },
                    DetailField {
                        rows: Some(3),
                        ..field(
                            "blocked_tools_json",
                            "工具拦截（JSON）",
                            "形如 {\"tool_name\": {\"blocked\": true}}；留空 = 不拦截工具名",
                            "textarea",
                        )
                    },
                ],
            },
        ],
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "detail.test.rs"]
mod tests;
