//! Agent（OAB bundle）详情页定义 —— 只读概览（`info` 绑定）
//!
//! 概览字段（版本/来源层级/安装目录）+ 内部实体计数（list_items
//! 下发的 `count_*`）+ `delete` 动作。「管理内部实体」入口由页面机制
//! 统一渲染（provider 声明 container_kinds 的条目，详情区顶部入口条），
//! 不属于本定义。

use crate::symbio_core::schemas::entities::{
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

/// Agent bundle 只读概览定义（纯静态结构，开销可忽略）
pub fn agent_detail_definition() -> DetailDefinition {
    DetailDefinition {
        binding: "info".into(),
        load_path: None,
        save_path: None,
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
                                label: "工作区级（.symbio/plugins/agent）".into(),
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
                title: Some("内部实体概览".into()),
                collapsed: false,
                fields: vec![
                    field("count_prompt", "提示词", "static"),
                    field("count_skill", "技能", "static"),
                    field("count_mcp", "MCP", "static"),
                ],
            },
        ],
        presets: None,
        badges: vec![],
        actions: vec![DetailAction {
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
        }],
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
        // 动作仅 delete：「管理内部实体」入口由页面机制统一渲染
        assert_eq!(def.actions.len(), 1);
        assert_eq!(def.actions[0].id, "delete");
    }
}
