//! `symbio/src/symbio_core/option.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[tokio::test]
async fn collect_without_parent_returns_empty() {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
    let v = collect_options(None, &ctx).await;
    assert!(v.list_option_fields().await.is_empty());
}

/// `order` 只影响**收集层**排序，不出现在结果里
#[tokio::test]
async fn option_fields_sort_by_order_and_drop_it() {
    let v = DefaultOptionVisitor::new();
    v.register_option_field(60, field("heartbeat", "form"))
        .await;
    v.register_option_field(10, field("workdir", "path")).await;
    v.register_option_field(40, field("risk_level", "select"))
        .await;

    let keys: Vec<String> = v
        .list_option_fields()
        .await
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, vec!["workdir", "risk_level", "heartbeat"]);
}

/// 同 key 覆盖（保留先注册槽位 ⇒ 序号也保留）
#[tokio::test]
async fn option_fields_dedupe_by_key_keeping_first_slot() {
    let v = DefaultOptionVisitor::new();
    v.register_option_field(20, field("agent_id", "select"))
        .await;
    v.register_option_field(10, field("workdir", "path")).await;
    // 同 key 覆盖：值换了，槽位（= 序号 20，排在 workdir 之后）不变
    v.register_option_field(99, field("agent_id", "text")).await;

    let list = v.list_option_fields().await;
    let keys: Vec<&str> = list.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, vec!["workdir", "agent_id"]);
    assert_eq!(list[1].widget, "text", "后者覆盖前者");
}

/// 造一个最小字段（只关心 key / widget 的用例用）
fn field(key: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.to_string(),
        label: key.to_string(),
        widget: widget.to_string(),
        ..Default::default()
    }
}
