//! `crates/symbio-plugin-mcp/src/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn definition_covers_mcp_form_surface() {
    let def = mcp_detail_definition();
    assert_eq!(def.binding, "upload");
    // 右栏标题靠 title_from=name（name 由 Read 在读时注入 = 条目 id）。
    // 这条断言守住「选中已有 Server 时右栏显示真实标题，而不回退「新建 MCP Server」」。
    assert_eq!(def.title_from, vec!["name".to_string()]);
    // 基本分区 + 工具过滤折叠分区
    assert_eq!(def.sections[0].fields.len(), 9);
    assert_eq!(def.sections[1].title.as_deref(), Some("工具过滤"));
    assert!(def.sections[1].collapsed);
    // 传输类型联动：stdio / 远程字段按 visible_when 互斥
    let stdio_count = def.sections[0]
        .fields
        .iter()
        .filter(|f| f.visible_when.as_ref().is_some_and(|c| c.equals.is_some()))
        .count();
    let remote_count = def.sections[0]
        .fields
        .iter()
        .filter(|f| {
            f.visible_when
                .as_ref()
                .is_some_and(|c| c.not_equals.is_some())
        })
        .count();
    assert_eq!(stdio_count, 3); // command / args / env
    assert_eq!(remote_count, 2); // url / headers（timeout_secs 两种传输共用，不在此列）
                                 // 传输类型与超时都有 default：新建时自动预填，不必手选/手填
    let ty = def.sections[0]
        .fields
        .iter()
        .find(|f| f.key == "type")
        .unwrap();
    assert_eq!(ty.default, Some(serde_json::json!("stdio")));
    let timeout = def.sections[0]
        .fields
        .iter()
        .find(|f| f.key == "timeout_secs")
        .unwrap();
    assert_eq!(timeout.default, Some(serde_json::json!(30)));
    // 动作：test/save/delete 全齐
    let ids: Vec<&str> = def.actions.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"test") && ids.contains(&"save") && ids.contains(&"delete"));

    // 导入整包是**详情页动作**（不是 `new_type` 上的字段）：只在草稿态出现，
    // 载荷声明为一个本地 zip——使用方据此先取文件再执行。
    let import = def
        .actions
        .iter()
        .find(|a| a.id == symbio_plugin_api::VDFS_ACTION_IMPORT)
        .expect("详情页要给出导入入口，否则草稿上无从导入");
    assert_eq!(
        import.pack.as_deref(),
        Some(symbio_plugin_api::VDFS_EXT_ZIP)
    );
    let when = import.when.as_ref().expect("导入只在草稿态");
    assert!(when.holds(&serde_json::json!({ "is_existing": false })));
    assert!(!when.holds(&serde_json::json!({ "is_existing": true })));
}
