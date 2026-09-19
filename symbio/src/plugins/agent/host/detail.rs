//! Agent 详情页定义 —— 只读概览（`info` 绑定）
//!
//! 概览字段（版本/来源层级/安装目录/已装能力目录）+ `open-container` /
//! `export` / `delete` 动作。「装了哪些能力」由**目录**回答（§4.1），不按类别
//! 点数——点数是 v1 的做法，与真实的能力来源两份真相。
//! 「管理内部条目」入口由页面机制
//! 统一渲染（provider 声明 container_kinds 的条目，详情区顶部入口条），
//! 不属于本定义。

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
                            },
                            DetailOption {
                                value: "global".into(),
                                label: "全局级（系统目录）".into(),
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
            // `export_zip`，与「新建类型 zip」的导入互为逆向。
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
mod tests {
    use super::*;

    #[test]
    fn definition_is_info_overview_with_container_entry() {
        let def = agent_detail_definition();
        assert_eq!(def.binding, "info");
        // 全部字段为 static 只读
        assert!(def
            .sections
            .iter()
            .all(|s| s.fields.iter().all(|f| f.widget == "static")));
        // 动作 = 浏览内部 + 导出 + 删除：「浏览内部」由本定义声明（VDFS 侧经
        // `open-container` 进入条目同名目录），取代原先由页面统一渲染的入口；
        // 「导出」是 VDFS 节点动作 `export` 的声明式入口
        assert_eq!(def.actions.len(), 3);
        assert_eq!(def.actions[0].id, "open-container");
        assert_eq!(
            def.actions[1].id,
            crate::symbio_core::vdfs_provider::VDFS_ACTION_EXPORT
        );
        assert_eq!(def.actions[2].id, "delete");
    }
}
