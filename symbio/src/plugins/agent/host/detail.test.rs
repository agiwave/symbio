//! `symbio/src/plugins/agent/host/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::symbio_core::vdfs_provider::{VDFS_ACTION_EXPORT, VDFS_ACTION_IMPORT};

#[test]
fn definition_is_info_overview_with_container_entry() {
    let def = agent_detail_definition();
    assert_eq!(def.binding, "info");
    // 全部字段为 static 只读
    assert!(def
        .sections
        .iter()
        .all(|s| s.fields.iter().all(|f| f.widget == "static")));
    // 动作 = 导入整包 + 浏览内部 + 导出 + 删除：「导入整包」是本定义的
    // **创建入口**（草稿态唯一可用），「浏览内部」由本定义声明（VDFS 侧经
    // `open-container` 进入条目同名目录），「导出」是 VDFS 节点动作 `export`
    // 的声明式入口。
    let ids: Vec<&str> = def.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            VDFS_ACTION_IMPORT,
            "open-container",
            VDFS_ACTION_EXPORT,
            "delete"
        ]
    );
}

/// **导入是草稿态专属**——同一份定义服务两种态，差别由 `when` 表达。
///
/// 这条是本定义的关键不变式：草稿上「导入整包」是新建智能体的唯一入口；
/// 落成后不再提供（要换内容就删掉重导，否则「导到哪个条目上」是个含混问题）。
/// 条件写反的后果是一目了然的：落成页上多出一个点了会覆盖自己的按钮。
#[test]
fn import_is_offered_in_draft_only() {
    let def = agent_detail_definition();
    let import = def
        .actions
        .iter()
        .find(|a| a.id == VDFS_ACTION_IMPORT)
        .expect("详情页要给出导入入口，否则草稿上无从导入");

    let when = import.when.as_ref().expect("导入只在草稿态");
    assert!(when.holds(&serde_json::json!({ "is_existing": false })));
    assert!(!when.holds(&serde_json::json!({ "is_existing": true })));

    assert_eq!(
        import.pack.as_deref(),
        Some(crate::symbio_core::vdfs_provider::VDFS_EXT_ZIP),
        "导入的载荷是一个本地 zip：使用方据此取文件"
    );
    assert!(
        import.disabled_when.is_none(),
        "草稿上它是唯一可做的事，不该再挂禁用条件"
    );
}
