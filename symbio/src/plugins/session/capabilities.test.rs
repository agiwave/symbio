//! `capabilities` 收集器（工具能力 + 选项）的单元测试。
//!
//! 与实现**同级**分文件（`capabilities.rs` + `capabilities.test.rs`）：
//! 实现只保留生产代码，测试全部放本文件。

use super::*;

use crate::symbio_core::capability_option_fields;

// ==================== 选项收集管线（宿主侧） ====================
//
// 原在 `symbio_core/capability/option.test.rs`，随 `collect_options` 一起搬来
// （先到 `session/options.rs`，后随收集器合一迁到本文件）。
// 测的是**无父插件时的降级**，不是契约——契约由 `capability/option.rs` 的读协议文档钉住。
//
// 选项行本身的两条（排序 / 去重）是表语义，守门人随 4e 迁到
// `providers/collectors/tool_visitor.test.rs`（全表共用的唯一实现）。

#[tokio::test]
async fn collect_without_parent_returns_empty() {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    let v = collect_options(None, &ctx).await;
    assert!(capability_option_fields(&*v).await.is_empty());
}
