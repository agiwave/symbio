//! actor 插件的单元测试：登记 / 内省 / 停用退化。
//!
//! ## 为什么每个用例都拿一把锁
//!
//! Actor 表是**进程级**的（`inventory` + `OnceLock`），而 cargo 并行跑测试。
//! 不互斥的话，"A 清表"会落在"B 断言"中间，失败随机出现。
//! 统一经 `actor_test_guard()` 把「清表 → 装配插件 → 断言」整段变成临界区。
//!
//! ## 为什么不用 `#[tokio::test]`
//!
//! 那把锁是 `std::sync::Mutex`，而 `#[tokio::test]` 的函数体是 async——
//! guard 会跨 `.await` 持有，clippy 的 `await_holding_lock` 会（正确地）报错。
//! 解法不是 `#[allow]`（那是在关掉一条真的警告），而是把 await 搬进一个
//! 私有 runtime 的 `block_on`（见 [`run`]）："持锁"与"跑 async"因此是两段。

use super::*;
use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::{
    actor_get, actor_register, ActorScope, ActorSpec, PluginDir, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginPayload, PluginSimpleRequest, PATH,
};
use std::sync::Arc;

fn test_dir() -> PluginDir {
    PluginDir::at(
        std::env::temp_dir().join("symbio-test-actor"),
        PLUGIN_ID_ACTOR,
    )
}

/// 干净状态 + 全局串行闸。guard 必须活到用例结束。
fn setup() -> std::sync::MutexGuard<'static, ()> {
    crate::symbio_core::actor_test_guard()
}

fn plugin(enabled: bool) -> Arc<ActorPlugin> {
    Arc::new(ActorPlugin::new(test_dir(), ActorConfig { enabled }))
}

fn ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!("expected data, got {other:?}"),
    }
}

/// 在**持锁期间**跑一段 async 路由调用（自建 current-thread runtime 的 `block_on`）。
///
/// 这样测试函数本身仍是同步的 —— `MutexGuard` 不跨 `.await` 持有。
fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("测试 runtime")
        .block_on(f)
}

#[test]
fn enabled_registers_decider_row() {
    let _g = setup();
    let _p = plugin(true);
    let row = actor_get(ACTOR_NAME_SESSION_DECIDER).expect("启用时必须登记判定者行");
    assert_eq!(row.pattern, ActorPattern::Decider);
    assert_eq!(row.budget_ms, DECIDER_BUDGET_MS);
    assert_eq!(row.scope, ActorScope::Root);
    assert_eq!(row.principal, "", "登记期用占位符，运行期解析");
}

#[test]
fn disabled_registers_nothing() {
    let _g = setup();
    let _p = plugin(false);
    assert!(
        actor_get(ACTOR_NAME_SESSION_DECIDER).is_none(),
        "停用时不登记——这正是 J2 的退化路径"
    );
}

/// `list` 路由必须同时看见**别的插件**登记的行 —— 表是共享的，插件不各持一份。
#[test]
fn list_route_returns_registered_rows() {
    let _g = setup();
    actor_register(&ActorScope::Root, ActorSpec::builtin("session.reasoner")).unwrap();
    let p = plugin(true);

    let v = run(async move { data_of(p.clone().route(ctx("list")).await.expect("list 必须可用")) });
    let rows = v["actors"].as_array().expect("actors 必须是数组");
    let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
    assert!(names.contains(&"session.reasoner"));
    assert!(names.contains(&ACTOR_NAME_SESSION_DECIDER));
}

#[test]
fn list_wire_shape_has_all_five_fields() {
    let _g = setup();
    let p = plugin(true);
    let v = run(async move { data_of(p.clone().route(ctx("list")).await.unwrap()) });
    let row = v["actors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == ACTOR_NAME_SESSION_DECIDER)
        .expect("判定者行必须在 list 里")
        .clone();

    assert_eq!(row["pattern"], "decider");
    assert_eq!(row["budget_ms"], 80);
    assert_eq!(row["scope"], "root");
    assert!(row["principal"].is_string(), "principal 字段必须在");
}

#[test]
fn disabled_makes_route_unreachable() {
    let _g = setup();
    let p = plugin(false);
    let err = run(async move { p.clone().route(ctx("list")).await.unwrap_err() });
    assert!(matches!(err, PluginError::NotFound(_)));
}

#[test]
fn unknown_route_is_not_found() {
    let _g = setup();
    let p = plugin(true);
    let err = run(async move { p.clone().route(ctx("nope")).await.unwrap_err() });
    assert!(matches!(err, PluginError::NotFound(_)));
}

#[test]
fn config_definition_has_one_form_section_with_one_toggle() {
    let d: DetailDefinition = config_definition();
    // 只有一个表单段、段内只有一个开关字段——本插件没有别的可配置面
    assert_eq!(d.sections.len(), 1);
    assert_eq!(d.sections[0].fields.len(), 1);
    assert_eq!(d.sections[0].fields[0].key, "enabled");
}

#[test]
fn metadata_id_matches_plugin_id() {
    assert_eq!(ActorPlugin::metadata().id, PLUGIN_ID_ACTOR);
}
