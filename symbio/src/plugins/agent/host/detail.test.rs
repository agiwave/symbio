//! `symbio/src/plugins/agent/host/detail.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
