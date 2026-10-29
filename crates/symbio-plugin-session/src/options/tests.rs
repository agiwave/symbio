//! `options` 模块的单元测试。
//!
//! 与实现**同级**分文件（`options/mod.rs` + 同级 `tests.rs`，见 `CONTRIBUTING.md`）：
//! `mod.rs` 只保留生产代码，测试全部放本文件。
//!
//! 这里锁的是「会话自有 3 项」这份**声明**：字段 key（= 会话解析链读取的 metadata
//! 键）、号段顺序（= UI 顺序）、以及各字段的 widget 与锁定/缺省语义。

use super::*;
// `traverse` 是 `Plugin` 的 trait 方法，需 trait 在作用域内才可解析
use symbio_core::Plugin;
// `ctx.set(...)` 来自 `PluginInvokeRequestExt`
use symbio_core::PluginInvokeRequestExt;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::test_dir;
use std::collections::BTreeSet;

fn plugin() -> SessionPlugin {
    use crate::config::SessionConfig;
    SessionPlugin::new(None, SessionConfig::default(), test_dir())
}

/// 字段声明：顺序 = 展示顺序，widget 决定紧凑形态怎么渲染
#[test]
fn session_option_fields_declare_the_option_bar_in_order() {
    let p = plugin();
    let declared: Vec<(i32, String, String)> = p
        .session_option_fields()
        .into_iter()
        .map(|(order, f)| (order, f.key, f.widget))
        .collect();

    assert_eq!(
        declared,
        vec![
            (ORDER_WORKDIR, "workdir".to_string(), "path".to_string()),
            (ORDER_RISK, "risk_level".to_string(), "select".to_string()),
            (ORDER_MODE, "mode".to_string(), "select".to_string()),
        ],
        "号段顺序即展示顺序；改号段等于改 UI，必须是有意的"
    );
}

/// 字段 key 就是**契约**：会话解析链（orchestrator / tool_executor）按这几个键
/// 从 `session.metadata` 回退取值。改 key = 静默失效（选项能选、能存，但没人读），
/// 故在这里钉死。
///
/// `heartbeat` 不在这张表里：它随调度搬进了 `crates/symbio-plugin-heartbeat`，那个键的**读方**是
/// 心跳插件（设置工具与调度器）而不是会话解析链，因此本插件不再声明它。
#[test]
fn field_keys_are_the_metadata_keys_the_parsers_read() {
    let keys: BTreeSet<String> = plugin()
        .session_option_fields()
        .into_iter()
        .map(|(_, f)| f.key)
        .collect();

    assert_eq!(
        keys.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["mode", "risk_level", "workdir"]
    );
}

/// 工作目录的锁定条件是**声明**（不是 Rust 算出来的布尔）：
/// 已绑定目录**且**已有对话历史 ⇒ 禁用。两个条件缺一不可——
/// 只看「有历史」会把「有历史但从未绑定目录」的会话也锁上。
///
/// 「有历史」用 `truthy` 而不是 `not_equals: 0`：草稿节点没有 `message_count`
/// 这个键，`truthy` 对缺席键求值为 false（可自由选择），`not_equals: 0` 求值为
/// true——那会让新建会话一上来就锁死工作目录。
#[test]
fn workdir_locks_only_when_bound_and_has_history() {
    let f = workdir_field();
    assert_eq!(f.widget, "path");
    assert_eq!(f.pick.as_deref(), Some(DETAIL_PICK_DIRECTORY));

    let cond = f.disabled_when.expect("必须声明锁定条件");
    let keys: Vec<&str> = cond.all.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, vec!["message_count", "workdir"]);
    assert_eq!(cond.all[0].truthy, Some(true));
    assert_eq!(cond.all[1].truthy, Some(true));

    // 未设置时的按钮文本来自**值→标签表**（`path` 没有候选菜单，`options` 退化为
    // 一张查表）：显示「未选择目录」而不是把字段名「工作目录」印上去。
    assert_eq!(
        f.options
            .iter()
            .map(|o| (o.value.as_str(), o.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("", "未选择目录")]
    );
}

/// 定义与会话无关：同一份声明算两次逐字节相同（`default` 表示后端缺省回落，
/// 而不是「当前值」——这正是新形态能同时服务草稿态与已落盘会话的原因）。
#[tokio::test]
async fn option_definition_is_session_independent() {
    let p = plugin();
    let a = serde_json::to_value(p.build_option_definition().await).unwrap();
    let b = serde_json::to_value(p.build_option_definition().await).unwrap();
    assert_eq!(a, b);
    assert_eq!(a["binding"], json!("option"));
}

/// 未装配容器 ⇒ 收集不到任何贡献方（会话自己也走同一次广播），定义退化为
/// **空字段表**——但定义本身不缺席：缺席会让前端把「草稿态」误当成「没有选项」。
#[tokio::test]
async fn option_definition_without_container_degrades_to_empty() {
    let p = plugin();
    let def = p.build_option_definition().await;
    assert_eq!(def.binding, "option");
    assert_eq!(def.sections.len(), 1);
    assert!(def.sections[0].fields.is_empty());
}

/// 会话作为**贡献方**：命中 `available_options` 时把字段注册进收集器。
///
/// 这是「会话自有 3 项」在生产路径上的唯一注册点——漏注册的表现是选项栏少了几项，
/// 而不会有任何编译错误。
#[tokio::test]
async fn session_contributes_option_fields_to_the_visitor() {
    use symbio_core::{
        capability_option_fields, PluginSimpleRequest, Registry, OPTION_VISITOR, PATH,
        TRAVERSE_AVAILABLE_OPTIONS,
    };
    use symbio_providers::DefaultToolVisitor;

    let p = Arc::new(plugin());
    let ctx = Arc::new(PluginSimpleRequest::new(None, None)).fork();
    ctx.set(PATH, TRAVERSE_AVAILABLE_OPTIONS.to_string());
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    ctx.set(OPTION_VISITOR, visitor.clone());

    p.traverse(String::new(), ctx).await.expect("traverse 成功");

    let keys: Vec<String> = capability_option_fields(&*visitor)
        .await
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, vec!["workdir", "risk_level", "mode"]);
}
