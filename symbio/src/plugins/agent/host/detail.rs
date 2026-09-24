//! Agent 详情页定义 —— 只读概览（`info` 绑定）
//!
//! 概览字段（版本/来源层级/安装目录/已装能力目录）+ `import` / `open-container` /
//! `export` / `delete` 动作。「装了哪些能力」由**目录**回答（§4.1），不按类别
//! 点数——点数是 v1 的做法，与真实的能力来源两份真相。
//! 「管理内部条目」入口由页面机制
//! 统一渲染（provider 声明 container_kinds 的条目，详情区顶部入口条），
//! 不属于本定义。
//!
//! **草稿（新建）态用的是同一份定义**：`root_new_type` 把本定义作为 `schema`
//! 下发，于是「点添加」与「选中一项」进的是同一张页。差别由动作的 `when` /
//! `disabled_when` 表达——草稿上只有「导入整包」可用（它正是新建智能体的
//! 唯一入口），其余动作要么隐藏（`open-container`）要么禁用（导出 / 删除）。

use crate::symbio_core::schemas::detail::{
    DetailAction, DetailCondition, DetailDefinition, DetailField, DetailOption, DetailSection,
};

fn field(key: &str, label: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        widget: widget.into(),
        ..Default::default()
    }
}

/// agent 目录只读概览定义（纯静态结构，开销可忽略）
pub fn agent_detail_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "info".into(),
        title_from: vec![],
        title_fallback: Some("Agent".into()),
        subtitle_from: vec!["version".into(), "scope".into()],
        name_from: vec![],
        id_from: vec![],
        sections: vec![
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![
                    field("version", "版本", "static"),
                    DetailField {
                        options: vec![
                            DetailOption {
                                value: "workspace".into(),
                                label: "工作区级（<本插件目录>）".into(),
                                description: None,
                            },
                            DetailOption {
                                value: "global".into(),
                                label: "全局级（系统目录）".into(),
                                description: None,
                            },
                        ],
                        ..field("scope", "来源层级", "static")
                    },
                    field("dir", "安装目录", "static"),
                ],
            },
            DetailSection {
                title: Some("已装能力".into()),
                collapsed: false,
                fields: vec![field("capabilities", "能力目录", "static")],
            },
        ],
        presets: None,
        badges: vec![],
        actions: vec![
            // 「导入整包」：VDFS 节点动作 `import`（vdfs/action）——**只在草稿
            // （新建）态**出现：条目 id 取自包内 manifest，落成后无从再导（要换
            // 内容就删掉重导）。它是「新建一个智能体」在详情页上的唯一入口，
            // 与「导出」「删除」同级——不是 `root_new_type` 上的一个字段。
            DetailAction {
                id: crate::symbio_core::vdfs_provider::VDFS_ACTION_IMPORT.into(),
                label: "导入整包".into(),
                style: "primary".into(),
                when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                // 载荷是一个本地 zip：使用方先取文件再执行本动作
                pack: Some(crate::symbio_core::vdfs_provider::VDFS_EXT_ZIP.into()),
                busy_label: Some("导入中…".into()),
                ..Default::default()
            },
            // 「浏览内部」入口：agent 目录内部（提示词 / 技能 / MCP）在 VDFS 上是
            // 条目同名目录下的子类别（VDFS 容器寻址），
            // 由页面层 `enter(节点路径)` 进入——取代原容器页。
            DetailAction {
                id: "open-container".into(),
                label: "浏览内部".into(),
                style: "primary".into(),
                payload: Some(serde_json::json!({ "kind": "agent" })),
                ..Default::default()
            },
            // 「导出」：VDFS 节点动作 `export`（vdfs/action）→ provider 的
            // `export_zip`，与「导入整包」互为逆向。
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
                id: "delete".into(),
                label: "删除该 Agent".into(),
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

#[cfg(test)]
#[path = "detail.test.rs"]
mod tests;
