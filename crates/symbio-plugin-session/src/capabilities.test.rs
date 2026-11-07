//! `capabilities` 收集器（工具能力 + 选项）的单元测试。
//!
//! 与实现**同级**分文件（`capabilities.rs` + `capabilities.test.rs`）：
//! 实现只保留生产代码，测试全部放本文件。

use super::*;

use symbio_sdk::capability::capability_option_fields;

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
        Arc::new(symbio_kernel::plugin::PluginSimpleRequest::new(None, None));
    let v = collect_options(None, &ctx).await;
    assert!(capability_option_fields(&*v).await.is_empty());
}

#[test]
fn bound_agent_without_published_scope_is_rejected() {
    let err = require_agent_scope(Some("t84-zero"), None)
        .expect_err("选中了智能体却没有作用域，必须拒绝本轮");
    assert!(
        err.to_string().contains("t84-zero"),
        "错误要能指认是哪个智能体：{err}"
    );
}

#[test]
fn no_agent_bound_may_run_in_host_mode() {
    assert!(require_agent_scope(None, None)
        .expect("没选智能体是合法宿主态")
        .is_none());
}

#[test]
fn published_scope_is_passed_through() {
    let scope = AgentScope {
        id: "t84-zero".into(),
        root: std::path::PathBuf::from("/tmp/agent/t84-zero"),
        permissions: Default::default(),
    };
    let got = require_agent_scope(Some("t84-zero"), Some(scope.clone())).expect("作用域已发布");
    assert_eq!(got.expect("Some").id, scope.id);
}

/// 「槽空 ≠ 不限制」的真值表（**不依赖并发**）：只有**没选智能体**才是宿主态，
/// 此时 `None` 合法；一旦会话绑定了智能体而作用域槽为空，`None` 就是「该拒」，
/// 不是「没有限制」的同义词。
///
/// 放在本层（会话）而不是 `net_guard`：只有这里知道「会话绑定了谁」。
/// `net_guard` 收到的 `None` 按契约是宿主态，它看不到绑定关系，在那里判会把
/// 宿主态一并判死。
#[test]
fn empty_slot_is_not_unrestricted_when_an_agent_is_bound() {
    let scope = AgentScope {
        id: "t84-zero".into(),
        root: std::path::PathBuf::from("/tmp/agent/t84-zero"),
        permissions: Default::default(),
    };
    // (绑定, 已发布作用域, 本轮是否允许)
    let cases: [(Option<&str>, Option<AgentScope>, bool); 4] = [
        (None, None, true),
        (None, Some(scope.clone()), true),
        (Some("t84-zero"), None, false),
        (Some("t84-zero"), Some(scope.clone()), true),
    ];
    for (bound, published, want_ok) in cases {
        let got = require_agent_scope(bound, published);
        assert_eq!(
            got.is_ok(),
            want_ok,
            "bound={bound:?} 的放行判定翻转——槽空被当成了不限制"
        );
    }
}
