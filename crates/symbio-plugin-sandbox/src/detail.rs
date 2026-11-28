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
pub(crate) const ACTION_COPY: &str = "copy";

pub(crate) fn sandbox_option_field(
    archives: &[(String, String)],
    active: Option<&str>,
    warning: Option<&str>,
    has_snapshot: bool,
) -> DetailField {
    let mut options = archives
        .iter()
        .filter(|(id, _)| !id.is_empty())
        .map(|(id, name)| DetailOption {
            value: id.clone(),
            label: name.clone(),
            description: None,
        })
        .collect::<Vec<_>>();
    if options.is_empty() {
        options.push(DetailOption {
            value: "default".into(),
            label: "默认".into(),
            description: None,
        });
    }
    if let Some(active) = active.filter(|id| !id.is_empty()) {
        if !archives.iter().any(|(id, _)| id == active) {
            options.push(DetailOption {
                value: active.to_string(),
                label: format!("{active}（档案缺失或不可用）"),
                description: Some("请重新选择可用档案，或选择「不限制」".to_string()),
            });
        }
    }
    let mut description =
        "选择本次会话的安全运行环境与审批策略；未选择时使用「默认」，显式选「不限制」才关闭沙箱限制".to_string();
    if let Some(warning) = warning.filter(|value| !value.is_empty()) {
        let impact = if has_snapshot {
            "当前继续使用上一份良好快照"
        } else {
            "当前没有可用快照，沙箱执行将失败关闭；请修复档案后重试"
        };
        description.push_str(&format!("。警告：{warning}，{impact}"));
    }
    DetailField {
        key: "sandbox_id".to_string(),
        label: "沙箱".to_string(),
        description: Some(description),
        widget: "select".to_string(),
        icon: Some("sandbox".to_string()),
        default: Some(serde_json::json!(active
            .filter(|id| !id.is_empty())
            .unwrap_or("default"))),
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
    let kinds = [
        ("shell_command", "完整命令"),
        ("shell_subcommand", "子命令"),
        ("shell_executable", "可执行程序"),
        ("shell_redirection_target", "重定向目标"),
        ("shell_unresolved_argument", "未解析参数"),
        ("shell_unparseable", "无法解析命令"),
        ("shell_expansion", "命令展开表达式"),
        ("vdfs_path", "路径"),
        ("url", "URL"),
        ("mcp_tool", "MCP 工具"),
        ("mcp_argument", "MCP 参数"),
        ("agent_id", "子智能体"),
        ("session_id", "会话"),
        ("heartbeat_action", "心跳动作"),
    ];
    let kind_opts: Vec<DetailOption> = kinds
        .iter()
        .map(|(value, label)| DetailOption {
            value: (*value).into(),
            label: (*label).into(),
            description: None,
        })
        .collect();
    let mode_opts: Vec<DetailOption> = [
        ("exact", "精确：值必须完全相等"),
        ("prefix", "前缀：命令按原文前缀；路径按目录段匹配"),
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
        ("low", "低风险（最低风险）"),
        ("medium", "中风险（最低风险）"),
        ("high", "高风险（最低风险）"),
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
            DetailColumn {
                placeholder: Some("选择或输入事实种类".into()),
                ..col("kind", "对象", "datalist", kind_opts)
            },
            col("match", "匹配", "select", mode_opts),
            DetailColumn {
                placeholder: Some("要匹配的值，如 git 或 /home/me/docs/**".into()),
                ..col("value", "值", "text", vec![])
            },
            col("action", "动作", "select", action_opts),
            col("risk", "风险下限", "select", risk_opts),
        ],
        ..field(
            "rules",
            "能力约束规则",
            "按顺序逐条匹配，首个命中生效：拒绝直接拦下；允许规则的风险档是工具风险的下限（最低风险），不得调低工具风险。路径规则存在时，shell 无法判定的参数默认拒绝，需另加「未解析参数」规则放行。留空整张表 = 不限制。",
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
                        default: Some(serde_json::json!(
                            "仅按工具声明的事实执行授权；不提供容器、网络或进程隔离。"
                        )),
                        ..field(
                            "boundary",
                            "能力边界",
                            "规则命中与审批不等于执行隔离；Shell、文件和出站请求仍运行在宿主权限下",
                            "static",
                        )
                    },
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
                        // 已有 default（interactive），无需再标必填——否则表单显示
                        // 必填星号却根本不需要用户填，徒增"必须理解该术语"的压力。
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
                        // 已有 default（medium），不标必填，理由同 approval_mode。
                        options: vec![
                            opt("low", "低（仅低风险自动放行）"),
                            opt("medium", "中（中风险及以下自动放行）"),
                            opt("high", "高（高风险及以下自动放行）"),
                        ],
                        default: Some(serde_json::json!("medium")),
                        ..field(
                            "risk_threshold",
                            "风险阈值",
                            "最大自动放行风险；高于阈值需审批",
                            "select",
                        )
                    },
                    DetailField {
                        default: Some(serde_json::json!(true)),
                        ..field(
                            "allow_session_approval",
                            "允许本会话免问",
                            "开启后，本会话可以选择不高于此沙箱阈值的风险档自动放行",
                            "toggle",
                        )
                    },
                    DetailField {
                        default: Some(serde_json::json!(true)),
                        ..field(
                            "allow_permanent_approval",
                            "允许永久免问",
                            "开启后，可保存不高于此沙箱阈值的跨会话授权；严格沙箱建议关闭",
                            "toggle",
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
                id: ACTION_COPY.into(),
                label: "复制为可编辑档案".into(),
                style: "secondary".into(),
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("复制中…".into()),
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
