//! `options` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `options.rs` 只保留生产代码，测试全部放本文件。
//!
//! 这里锁的是「会话自有 4 项」这份**声明**：字段 key（= 会话解析链读取的 metadata
//! 键）、号段顺序（= UI 顺序）、以及各字段的 widget 与锁定/缺省语义。

use super::*;
// `traverse` 是 `Plugin` 的 trait 方法，需 trait 在作用域内才可解析
use crate::symbio_core::Plugin;
// `ctx.set(...)` 来自 `PluginInvokeRequestExt`
use crate::symbio_core::PluginInvokeRequestExt;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::plugins::session::test_dir;
use std::collections::BTreeSet;

fn plugin() -> SessionPlugin {
    use crate::plugins::session::config::SessionConfig;
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
            (ORDER_HEARTBEAT, "heartbeat".to_string(), "form".to_string()),
        ],
        "号段顺序即展示顺序；改号段等于改 UI，必须是有意的"
    );
}

/// 字段 key 就是**契约**：会话解析链（orchestrator / tool_executor）按这几个键
/// 从 `session.metadata` 回退取值。改 key = 静默失效（选项能选、能存，但没人读），
/// 故在这里钉死。
#[test]
fn field_keys_are_the_metadata_keys_the_parsers_read() {
    let keys: BTreeSet<String> = plugin()
        .session_option_fields()
        .into_iter()
        .map(|(_, f)| f.key)
        .collect();

    assert_eq!(
        keys.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["heartbeat", "mode", "risk_level", "workdir"]
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

/// 心跳：字段自带子定义（`form` widget），且**基础设置恒可见**。
#[test]
fn heartbeat_field_carries_its_sub_definition_and_summary() {
    let f = heartbeat_field();
    assert_eq!(f.widget, "form");
    let form = f.form.expect("心跳子定义存在");

    let keys: Vec<&str> = form.sections[0]
        .fields
        .iter()
        .map(|x| x.key.as_str())
        .collect();
    assert_eq!(
        keys,
        vec!["enabled", "interval_seconds", "prompt", "include_history"]
    );
    // 基础设置必须恒可见（不得带 visible_when）：否则未启用时表单只剩开关，
    // 用户看不到任何可填参数（历史回归）。
    for sub in &form.sections[0].fields {
        assert!(
            sub.visible_when.is_none(),
            "字段 {} 不应带显隐门控",
            sub.key
        );
    }

    // 紧凑形态的一句话摘要：代表值取子定义的 title_from（enabled），
    // 再按本字段 options 查「值→标签」⇒ 未启用 = 未开启
    assert_eq!(form.title_from, vec!["enabled".to_string()]);
    let labels: Vec<(&str, &str)> = f
        .options
        .iter()
        .map(|o| (o.value.as_str(), o.label.as_str()))
        .collect();
    assert_eq!(labels, vec![("true", "已开启"), ("false", "未开启")]);

    // 未配置过心跳时紧凑形态要显示「未开启」而不是字段名：这靠 `default`
    // （子对象缺省值）给出 `enabled = false`，再按 `title_from` + `options` 查表。
    let d = f.default.as_ref().expect("必须给子对象缺省值");
    assert_eq!(d["enabled"], json!(false));
    assert_eq!(d["interval_seconds"], json!(DEFAULT_HEARTBEAT_INTERVAL));
    assert_eq!(d["prompt"], json!(""));
    assert_eq!(d["include_history"], json!(true));
}

/// 子表单里 `interval_seconds` 的 `default` 与子对象缺省值**同源**（都是
/// [`DEFAULT_HEARTBEAT_INTERVAL`]）：两处各写一个数字迟早会漂。
#[test]
fn heartbeat_interval_defaults_agree() {
    let form = heartbeat_field().form.expect("心跳子定义存在");
    let interval = form.sections[0]
        .fields
        .iter()
        .find(|x| x.key == "interval_seconds")
        .expect("interval_seconds 字段存在");
    assert_eq!(
        interval.default,
        Some(json!(DEFAULT_HEARTBEAT_INTERVAL)),
        "子表单字段缺省值必须与子对象缺省值一致"
    );
}

/// 「立即心跳」按钮**已取消** —— 防回归。
///
/// 它原是一个独立命令选项（图标 `play`，点了直接调 `session/heartbeat/trigger`）。
/// 取消的理由是**它的作用与「在输入框里直接发一条消息」完全重复**——心跳的实质
/// 就是往会话发一轮提示词，想立刻做一次直接在输入框发即可。若哪天有人把它加回来，
/// 同一件事就又有了两个入口，且按钮那个绕开了对话本身。
///
/// 注意这里锁的是**两件事**：触发按钮没了、**心跳配置仍在**（取消的只是那个
/// 手动触发入口，不是心跳任务本身）。
#[test]
fn heartbeat_trigger_option_is_gone() {
    let keys: Vec<String> = plugin()
        .session_option_fields()
        .into_iter()
        .map(|(_, f)| f.key)
        .collect();

    assert!(
        !keys.iter().any(|k| k.contains("trigger")),
        "「立即心跳」按钮不得被加回来：它与「输入框里直接发一条消息」重复"
    );
    assert!(
        keys.iter().any(|k| k == "heartbeat"),
        "心跳任务**配置**必须保留——取消的只是手动触发入口"
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
/// 这是「会话自有 4 项」在生产路径上的唯一注册点——漏注册的表现是选项栏少了几项，
/// 而不会有任何编译错误。
#[tokio::test]
async fn session_contributes_option_fields_to_the_visitor() {
    use crate::providers::collectors::DefaultOptionVisitor;
    use crate::symbio_core::{
        OptionVisitor, PluginSimpleRequest, OPTION_VISITOR, PATH, TRAVERSE_AVAILABLE_OPTIONS,
    };

    let p = Arc::new(plugin());
    let ctx = Arc::new(PluginSimpleRequest::new(None, None)).fork();
    ctx.set(PATH, TRAVERSE_AVAILABLE_OPTIONS.to_string());
    let visitor: Arc<dyn OptionVisitor> = Arc::new(DefaultOptionVisitor::new());
    ctx.set(OPTION_VISITOR, visitor.clone());

    p.traverse(String::new(), ctx).await.expect("traverse 成功");

    let keys: Vec<String> = visitor
        .list_option_fields()
        .await
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, vec!["workdir", "risk_level", "mode", "heartbeat"]);
}

// ==================== 收集管线（宿主侧） ====================
//
// 原在 `symbio_core/capability/option.test.rs`，随 `collect_options` 一起搬来。
// 测的是**无父插件时的降级**，不是契约——契约由 `OptionVisitor` 的文档钉住。
//
// 收集器**本身**的两条（排序 / 去重）随默认实现迁到
// `providers/collectors/option_visitor.test.rs`。

#[tokio::test]
async fn collect_without_parent_returns_empty() {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    let v = collect_options(None, &ctx).await;
    assert!(v.list_option_fields().await.is_empty());
}
