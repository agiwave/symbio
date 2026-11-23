//! 沙箱档案详情页定义 + 会话选项字段（definition-driven detail）。
//!
//! 表单与 `SandboxProfileConfig` 同构：name/description 是展示元数据，
//! approval_mode/risk_threshold 承接原会话级 mode/risk_level（裁决 B），
//! allowed_facts/blocked_tools 是沙箱内部开关。前端只按 widget 通用渲染，
//! **不理解任何沙箱概念**——与 mcp / model 的详情走同一条通道。

use symbio_plugin_api::{
    DetailAction, DetailColumn, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailSection,
};

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

/// 规则表格的列定义（沙箱插件**内部**声明，机制层只有通用 table widget）。
///
/// 一张表同时表达命令与目录规则：每行选「匹配对象」(`kind`)、匹配模式、值、
/// 判定动作与风险档。顺序判定、首个命中生效；某对象配了规则却全未命中 ⇒ 拒绝，
/// 要表达「只禁几条、其余放行」就加一条 `glob / * / 允许` 兜底行。
fn rule_table_field() -> DetailField {
    let kinds = ["shell_command", "vdfs_path", "url"];
    let kind_opts: Vec<DetailOption> = kinds
        .iter()
        .map(|k| DetailOption {
            value: (*k).into(),
            label: match *k {
                "shell_command" => "命令",
                "vdfs_path" => "路径",
                _ => "URL",
            }
            .into(),
            description: None,
        })
        .collect();
    let mode_opts: Vec<DetailOption> = [
        ("exact", "精确：值必须完全相等"),
        ("prefix", "前缀：值以填写内容开头"),
        ("contains", "包含：值含填写内容"),
        ("glob", "通配：* 任意长、? 单字符"),
    ]
    .iter()
    .map(|(v, l)| DetailOption {
        value: (*v).into(),
        label: (*l).into(),
        description: None,
    })
    .collect();
    let action_opts: Vec<DetailOption> = [("allow", "允许"), ("deny", "拒绝")]
        .iter()
        .map(|(v, l)| DetailOption {
            value: (*v).into(),
            label: (*l).into(),
            description: None,
        })
        .collect();
    let risk_opts: Vec<DetailOption> = [
        ("inherit", "继承工具自身判定（留空同义）"),
        ("low", "低风险（一般不触发审批）"),
        ("medium", "中风险"),
        ("high", "高风险（通常需要审批）"),
    ]
    .iter()
    .map(|(v, l)| DetailOption {
        value: (*v).into(),
        label: (*l).into(),
        description: None,
    })
    .collect();
    let col = |key: &str, label: &str, widget: &str, options: Vec<DetailOption>| DetailColumn {
        key: key.into(),
        label: label.into(),
        widget: widget.into(),
        options,
        default: None,
        placeholder: None,
    };
    DetailField {
        columns: vec![
            col("kind", "对象", "select", kind_opts),
            col("match", "匹配", "select", mode_opts),
            DetailColumn {
                placeholder: Some("要匹配的值，如 git 或 /home/me/docs/**".into()),
                ..col("value", "值", "text", vec![])
            },
            col("action", "动作", "select", action_opts),
            col("risk", "风险档", "select", risk_opts),
        ],
        ..field(
            "rules",
            "能力约束规则",
            "按顺序逐条匹配，首个命中生效：拒绝直接拦下；允许并可指定风险档（影响是否需审批）。留空整张表 = 不限制。",
            "table",
        )
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
        // 与 mcp / skill 同形：右栏标题下显示描述，列表卡片也有副标题来源。
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
                        placeholder: Some("例如：办公环境".into()),
                        ..field(
                            "name",
                            "名称",
                            "给这套安全设置起个名字；保存时自动生成档案标识",
                            "text",
                        )
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
                            "需要审批的操作如何处置：弹卡让你确认，或无人值守自动拒绝",
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
                            "越高越谨慎：更多操作会被判定为需要审批",
                            "select",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("能力约束规则（可选）".into()),
                collapsed: true,
                fields: vec![rule_table_field()],
            },
        ],
        // 与 mcp / skill 同形：详情动作显式声明，表单渲染器据此决定显示哪些按钮。
        // 早期这里用 `..Default::default()` 漏了 actions，导致沙箱编辑页没有「保存」。
        // 只读（内置档）节点的写动作由前端按访问位自动剥掉，与 mcp / skill 同一机制。
        actions: vec![
            DetailAction {
                id: "save".into(),
                label: "保存".into(),
                style: "primary".into(),
                busy_label: Some("保存中…".into()),
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
                label: "删除沙箱".into(),
                style: "icon danger".into(),
                // 草稿态（新建）没有目标可删，与 mcp / skill 一致。
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("删除中…".into()),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "detail.test.rs"]
mod tests;
