//! MCP 详情页定义（definition-driven detail）
//!
//! 设计基准 = `McpServerConfig`（server.json）的字段全集：
//! - 传输类型 select 联动字段显隐（stdio → command/args/env；
//!   http/sse → url/headers/timeout，经 `visible_when` 条件显隐）
//! - list / map 结构化控件（args 每行一项、env·headers 每行 KEY=VALUE，
//!   序列化约定见 DetailForm 渲染器，两侧一致）
//! - 工具白/黑名单折叠分区、启用开关
//! - 条件徽标（已停用）与动作（连接测试 / 保存 / 导入整包 / 导出整包 / 删除）
//!
//! 表单 manifest 与 server.json 同构（`type` = 传输类型，`name` 仅用于
//! 新建态派生目录 id，`validate_manifest` 时丢弃），校验与规范化在
//! `validate_manifest`（plugin.rs）完成。

use crate::symbio_core::schemas::detail::{
    DetailAction, DetailBadge, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailSection,
};

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

fn cond(
    key: &str,
    equals: Option<serde_json::Value>,
    not_equals: Option<serde_json::Value>,
) -> DetailCondition {
    DetailCondition {
        key: key.into(),
        equals,
        not_equals,
        ..Default::default()
    }
}

/// stdio 专属字段显隐条件（type == "stdio"）
fn when_stdio() -> DetailCondition {
    cond("type", Some(serde_json::json!("stdio")), None)
}

/// HTTP/SSE 专属字段显隐条件（type != "stdio"，两者共用 url/headers/timeout）
fn when_remote() -> DetailCondition {
    cond("type", None, Some(serde_json::json!("stdio")))
}

/// MCP 详情页定义（纯静态结构，开销可忽略）
pub fn mcp_detail_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "upload".into(),
        title_from: vec![],
        title_fallback: Some("新建 MCP Server".into()),
        subtitle_from: vec!["type".into(), "command".into(), "url".into()],
        name_from: vec!["name".into()],
        id_from: vec!["name".into()],
        sections: vec![
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![
                    DetailField {
                        required: true,
                        placeholder: Some("例如：github（即条目目录名）".into()),
                        ..field(
                            "name",
                            "名称",
                            "用于生成条目目录名（ID），不写入 server.json",
                            "text",
                        )
                    },
                    DetailField {
                        default: Some(serde_json::json!(true)),
                        ..field("enabled", "启用", "禁用后该 Server 的工具不注册", "toggle")
                    },
                    DetailField {
                        required: true,
                        options: vec![
                            opt("stdio", "本地进程 (stdio)"),
                            opt("http", "HTTP"),
                            opt("sse", "SSE"),
                        ],
                        ..field(
                            "type",
                            "传输类型",
                            "stdio 走本地子进程，http/sse 走远程端点",
                            "select",
                        )
                    },
                    DetailField {
                        required: true,
                        visible_when: Some(when_stdio()),
                        placeholder: Some("例如：npx".into()),
                        ..field(
                            "command",
                            "命令",
                            "stdio transport 启动的可执行命令",
                            "text",
                        )
                    },
                    DetailField {
                        visible_when: Some(when_stdio()),
                        rows: Some(3),
                        ..field("args", "命令参数", "每行一个参数，按顺序传给命令", "list")
                    },
                    DetailField {
                        visible_when: Some(when_stdio()),
                        rows: Some(3),
                        ..field("env", "环境变量", "每行一项：KEY=VALUE", "map")
                    },
                    DetailField {
                        required: true,
                        visible_when: Some(when_remote()),
                        placeholder: Some("https://example.com/mcp".into()),
                        ..field("url", "端点 URL", "HTTP / SSE transport 的服务端点", "text")
                    },
                    DetailField {
                        visible_when: Some(when_remote()),
                        rows: Some(3),
                        ..field(
                            "headers",
                            "自定义请求头",
                            "每行一项：Header=Value（Authorization 等鉴权头）",
                            "map",
                        )
                    },
                    DetailField {
                        visible_when: Some(when_remote()),
                        min: Some(1.0),
                        placeholder: Some("30".into()),
                        ..field(
                            "timeout_secs",
                            "请求超时（秒）",
                            "留空使用默认 30 秒；stdio 不使用此字段",
                            "number",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("工具过滤".into()),
                collapsed: true,
                fields: vec![
                    DetailField {
                        rows: Some(3),
                        ..field(
                            "include_tools",
                            "工具白名单",
                            "每行一个工具名；留空 = 全部工具",
                            "list",
                        )
                    },
                    DetailField {
                        rows: Some(3),
                        ..field(
                            "exclude_tools",
                            "工具黑名单",
                            "每行一个工具名；优先级高于白名单",
                            "list",
                        )
                    },
                ],
            },
        ],
        presets: None,
        badges: vec![DetailBadge {
            when: Some(cond("enabled", Some(serde_json::json!(false)), None)),
            label: "已停用".into(),
            style: "disabled".into(),
        }],
        actions: vec![
            DetailAction {
                id: "test".into(),
                label: "连接测试".into(),
                style: "secondary".into(),
                when: Some(cond(
                    "cap.test_connection",
                    Some(serde_json::json!(true)),
                    None,
                )),
                disabled_when: Some(cond("is_existing", Some(serde_json::json!(false)), None)),
                busy_label: Some("连接中…".into()),
                ..Default::default()
            },
            DetailAction {
                id: "save".into(),
                label: "保存".into(),
                style: "primary".into(),
                busy_label: Some("保存中…".into()),
                ..Default::default()
            },
            // 「导入整包」：VDFS 节点动作 `import`（vdfs/action）——**只在草稿
            // （新建）态**出现：条目名取自包的文件名，落成后无从再导（要换内容
            // 就删掉重导）。它是与「在表单里填」并列的另一条创建路，也是详情页
            // 的一条动作——不是 `new_type` 上的一个字段。
            DetailAction {
                id: crate::symbio_core::VDFS_ACTION_IMPORT.into(),
                label: "导入整包".into(),
                style: "secondary".into(),
                when: Some(cond("is_existing", Some(serde_json::json!(false)), None)),
                // 载荷是一个本地 zip：使用方先取文件再执行本动作
                pack: Some(crate::symbio_core::VDFS_EXT_ZIP.into()),
                busy_label: Some("导入中…".into()),
                ..Default::default()
            },
            // 「导出」：VDFS 节点动作 `export`（vdfs/action）→ provider 的
            // `export_zip`；与「导入整包」互为逆向
            DetailAction {
                id: crate::symbio_core::VDFS_ACTION_EXPORT.into(),
                label: "导出整包".into(),
                style: "secondary".into(),
                disabled_when: Some(cond("is_existing", Some(serde_json::json!(false)), None)),
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
                label: "删除 Server".into(),
                style: "icon danger".into(),
                disabled_when: Some(cond("is_existing", Some(serde_json::json!(false)), None)),
                busy_label: Some("删除中…".into()),
                ..Default::default()
            },
        ],
    }
}

#[cfg(test)]
#[path = "detail.test.rs"]
mod tests;
