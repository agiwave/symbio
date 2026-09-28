//! Agent 详情页定义 —— 只读概览（`info` 绑定）
//!
//! 概览分两段：**已装能力**（在前）+ **元数据**（在后）——「能力 > 元数据」：进这一页
//! 的人先想知道「这个智能体能干什么」。「装了哪些能力」由**目录**回答（§4.1），
//! 不按类别点数——点数会与真实的能力来源形成两份真相。
//!
//! 能力那段的三个字段都读 `agent_dir_info` 交出的**结构化**值
//! （`capability_count` / `capabilities`〔类别→条目数〕/ `capability_kinds`），
//! 不是一句在源头就拼接好的话——详情页才能按需摆放（用户要求：能后端驱动的
//! 优先后端驱动）。
//!
//! 另有 `import` / `open-container` / `export` / `delete` 动作。「管理内部条目」
//! 入口由页面机制统一渲染（provider 声明 container_kinds 的条目，详情区顶部
//! 入口条），不属于本定义。
//!
//! **草稿（新建）态用的是同一份定义**：根节点自述里的 `new_type`
//! （`VdfsNode::new_type`）把本定义作为 `schema` 下发，于是「点添加」与
//! 「选中一项」进的是同一张页。差别由动作的 `when` /
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
            // **能力在前、元数据在后**（「能力 > 元数据」）：进这一页的人先想知道
            // 「这个智能体能干什么」，而不是「它从哪装的、装在哪」。
            DetailSection {
                title: Some("已装能力".into()),
                collapsed: false,
                fields: vec![
                    // 一句话概括「装了几类」——`staticDisplay` 对数字直接显示
                    DetailField {
                        full_width: false,
                        ..field("capability_count", "能力类别数", "static")
                    },
                    // 类别 → 条目数（对象值，前端 `staticDisplay` 逐项展开为 `k v`）。
                    // 键来自目录本身、不预设：agent 目录挂了什么就列什么。
                    DetailField {
                        full_width: true,
                        description: Some("每一类下列出的条目数".into()),
                        ..field("capabilities", "各类能力条目数", "static")
                    },
                    // 类别名清单（数组值，前端以「、」连接）
                    DetailField {
                        full_width: true,
                        ..field("capability_kinds", "已装能力类别", "static")
                    },
                ],
            },
            DetailSection {
                title: Some("元数据".into()),
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
                    DetailField {
                        full_width: true,
                        ..field("dir", "安装目录", "static")
                    },
                ],
            },
        ],
        presets: None,
        badges: vec![],
        actions: vec![
            // 「导入整包」：VDFS 节点动作 `import`（vdfs/action）——**只在草稿
            // （新建）态**出现：条目 id 取自包内 manifest，落成后无从再导（要换
            // 内容就删掉重导）。它是「新建一个智能体」在详情页上的唯一入口，
            // 与「导出」「删除」同级——不是 `new_type` 上的一个字段。
            DetailAction {
                id: crate::symbio_core::VDFS_ACTION_IMPORT.into(),
                label: "导入整包".into(),
                style: "primary".into(),
                when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                // 载荷是一个本地 zip：使用方先取文件再执行本动作
                pack: Some(crate::symbio_core::VDFS_EXT_ZIP.into()),
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
                id: crate::symbio_core::VDFS_ACTION_EXPORT.into(),
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
